// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::{path::Path, sync::Arc};

use astarte_device_sdk::Client;
use edgehog_store::models::job::{Job, job_type::JobType, status::JobStatus};
use eyre::{Context, eyre};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument};

use crate::{
    controller::actor::Persisted,
    file_transfer::{
        config::FileTransferArgs, file_system::store::FileStorage, interface::file::StoredFile,
    },
    jobs::Queue,
    storage::{
        interface::FileStorageResponse,
        request::{CleanUp, FileStorageId},
    },
};

pub(crate) mod interface;
pub(crate) mod request;

pub struct Receiver<C> {
    queue: Queue,
    notify: Arc<Notify>,
    device: C,
}

impl<C> Receiver<C> {
    pub fn new(queue: Queue, notify: Arc<Notify>, device: C) -> Self {
        Self {
            queue,
            notify,
            device,
        }
    }
}

impl<C> Persisted for Receiver<C>
where
    C: Client + Send + Sync + 'static,
{
    type Msg = interface::DeleteFile;

    fn task() -> &'static str {
        "storage"
    }

    async fn init(&mut self) -> eyre::Result<()> {
        Ok(())
    }

    fn queue(&self) -> &crate::jobs::Queue {
        &self.queue
    }

    fn workers(&self) -> &tokio::sync::Notify {
        &self.notify
    }

    async fn validate_job(&mut self, msg: &Self::Msg) -> eyre::Result<Job> {
        let delete = request::Delete::try_from(msg)?;

        Job::try_from(delete)
    }

    async fn fail_job(&mut self, msg: &Self::Msg, report: eyre::Report) {
        let send = FileStorageResponse::validation_error(&msg.id, Self::Msg::RESPONSE_TYPE, report)
            .send(&mut self.device)
            .await;

        if let Err(error) = send {
            error!(%error, "response send error");
        }
    }

    async fn handle_backpressure(&mut self, msg: &Self::Msg) {
        let send = FileStorageResponse::busy_error(&msg.id, Self::Msg::RESPONSE_TYPE)
            .send(&mut self.device)
            .await;

        if let Err(error) = send {
            error!(%error, "response send error");
        }
    }
}

/// Task that handles scheduled task and requested task for the storage
pub struct StorageTask<C> {
    storage: FileStorage<()>,
    queue: Queue,
    notify: Arc<Notify>,
    device: C,
}

impl<C> StorageTask<C> {
    pub(crate) fn new(
        args: FileTransferArgs,
        queue: Queue,
        notify: Arc<Notify>,
        device: C,
    ) -> Self {
        Self {
            storage: FileStorage::new(args.storage_dir),
            queue,
            notify,
            device,
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn run(mut self, cancel: CancellationToken) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        loop {
            self.jobs(&cancel).await?;

            if !self.wait_next(&cancel).await? {
                break;
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn wait_next(&self, cancel: &CancellationToken) -> eyre::Result<bool> {
        debug!("waiting for next job");

        if cancel.is_cancelled() {
            return Ok(false);
        }

        if self
            .queue
            .contains(JobType::FileStorage, StorageJobTag::Delete.into())
            .await?
        {
            return Ok(true);
        }

        if let Some(timestamp) = self.queue.next_schedule(JobType::FileStorage).await? {
            let Some(instant) = timestamp.wait_until()? else {
                return Ok(true);
            };

            let sleep = tokio::time::sleep_until(instant.into());

            tokio::select! {
                () = sleep => {
                    Ok(true)
                }
                () = self.notify.notified() => {
                    Ok(true)
                }
                () = cancel.cancelled() => {
                    Ok(false)
                }
            }
        } else {
            debug!("no queued job");

            Ok(cancel
                .run_until_cancelled(self.notify.notified())
                .await
                .is_some())
        }
    }

    #[instrument(skip_all)]
    async fn jobs(&mut self, cancel: &CancellationToken) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        while !cancel.is_cancelled()
            && let Some(job) = self
                .queue
                .next_job_tag(JobType::FileStorage, StorageJobTag::Delete.into())
                .await?
        {
            self.handle(job).await?;
        }

        while !cancel.is_cancelled()
            && let Some(job) = self.queue.next_scheduled_job(JobType::FileStorage).await?
        {
            if let Err(error) = self.cleanup(CleanUp::try_from(&job)?).await {
                error!(%error,"couldn't handle storage job");
            }
        }

        Ok(())
    }

    async fn handle(&mut self, job: Job) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        let delete = request::Delete::try_from(&job)?;
        let store = delete.id();

        let result = self.delete(delete).await;

        match result {
            Ok(_) => self.job_done(store).await,
            Err(error) => {
                error!(
                    error = format!("{error:#}"),
                    "couldn't perform requested storage operation"
                );

                self.job_error(store, error).await
            }
        }
    }

    async fn cleanup(&mut self, job: CleanUp<'_>) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        Self::inner_delete(job.file_path, false).await?;

        StoredFile::deleted(job.id, &mut self.device).await;

        // Just delete the job since we don't need to do anything for completion
        self.queue
            .delete(&job.id, JobType::FileStorage, StorageJobTag::CleanUp.into())
            .await?;

        Ok(())
    }

    async fn delete(&mut self, job: request::Delete) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        let path = self.storage.file_path(&job.file_id);

        Self::inner_delete(path, job.force).await?;

        StoredFile::deleted(job.file_id, &mut self.device).await;

        Ok(())
    }

    async fn inner_delete(file_path: impl AsRef<Path>, _force: bool) -> eyre::Result<()> {
        let file_path = file_path.as_ref();

        if file_path.is_dir() {
            tokio::fs::remove_dir_all(&file_path)
                .await
                .wrap_err("couldn't remove directory")?;
        } else {
            tokio::fs::remove_file(&file_path)
                .await
                .wrap_err("couldn't remove file")?;
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn job_done(&mut self, store: FileStorageId) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        let tag = StorageJobTag::from(store.ty);

        self.queue
            .update(&store.id, JobType::FileStorage, tag.into(), JobStatus::Done)
            .await
            .wrap_err("couldn't update job status")?;

        info!("job success");

        FileStorageResponse::success(store.clone())
            .send(&mut self.device)
            .await
            .wrap_err("couldn't send response to astarte")?;

        self.queue
            .delete(&store.id, JobType::FileStorage, tag.into())
            .await
            .wrap_err("couldn't clean up job")?;

        Ok(())
    }

    async fn job_error(&mut self, store: FileStorageId, error: eyre::Report) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        let tag = StorageJobTag::from(store.ty);
        self.queue
            .update(
                &store.id,
                JobType::FileStorage,
                tag.into(),
                JobStatus::Error,
            )
            .await
            .wrap_err("couldn't update job status")?;

        error!("job error");

        FileStorageResponse::runtime_error(store.clone(), error)
            .send(&mut self.device)
            .await
            .wrap_err("couldn't send response to astarte")?;

        self.queue
            .delete(&store.id, JobType::FileStorage, tag.into())
            .await
            .wrap_err("couldn't clean up job")?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum StorageJobTag {
    CleanUp = 0,
    Delete = 1,
}

impl From<StorageJobTag> for i32 {
    fn from(value: StorageJobTag) -> Self {
        value as i32
    }
}

impl TryFrom<i32> for StorageJobTag {
    type Error = eyre::Report;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(StorageJobTag::CleanUp),
            1 => Ok(StorageJobTag::Delete),
            _ => Err(eyre!("unrecognize file transfer job tag {value}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        storage::{interface::ActionType, request::Delete},
        tests::with_insta,
    };
    use astarte_device_sdk::{
        AstarteData, aggregate::AstarteObject, pairing::api::PairingApi, store::SqliteStore,
        transport::mqtt::Mqtt,
    };
    use astarte_device_sdk_mock::MockDeviceClient;
    use edgehog_store::db::Handle;
    use mockall::predicate;
    use rstest::{Context, rstest};
    use tempdir::TempDir;
    use tokio::{fs::OpenOptions, io::AsyncWriteExt};
    use uuid::Uuid;

    use super::*;

    #[rstest]
    #[case(StorageJobTag::CleanUp)]
    #[case(StorageJobTag::Delete)]
    fn job_tag_roundtrip(#[context] ctx: Context, #[case] value: StorageJobTag) {
        let buf = i32::from(value);

        let res = StorageJobTag::try_from(buf).unwrap();

        assert_eq!(res, value);

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, format!("{value:?} = {}", buf));
        });
    }

    async fn mk_storage<C>(dir: impl AsRef<Path>, device: C) -> StorageTask<C> {
        let db_file = dir.as_ref().join("state.db");

        let args = FileTransferArgs::with_store_dir(None, dir.as_ref());
        let queue = Queue::new(Handle::open(db_file).await.unwrap());
        let notify = Arc::new(Notify::new());

        StorageTask::new(args, queue, notify, device)
    }

    #[tokio::test]
    async fn test_delete_job() {
        let dir = TempDir::new("delete_job").unwrap();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();

        let request_id = Uuid::new_v4();
        let file_id = Uuid::new_v4();

        device
            .expect_unset_property()
            .times(2)
            .with(
                predicate::eq("io.edgehog.devicemanager.storage.File"),
                predicate::function(move |path: &str| path.starts_with(&format!("/{}", file_id))),
            )
            .returning(|_, _| Ok(()));

        device
            .expect_send_object()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.storage.Response"),
                predicate::eq("/request"),
                predicate::function(move |obj: &AstarteObject| {
                    let id_ok = obj.get("id") == Some(&AstarteData::String(request_id.to_string()));
                    let code_ok = obj.get("code") == Some(&AstarteData::Integer(0));
                    let type_ok = obj.get("type")
                        == Some(&AstarteData::String(ActionType::Delete.to_string()));

                    id_ok && code_ok && type_ok
                }),
            )
            .returning(|_, _, _| Ok(()));

        let mut storage = mk_storage(dir.path(), device).await;

        let store_dir = dir.path().join("file-store");
        let file_path = store_dir.join(file_id.to_string());
        tokio::fs::create_dir_all(store_dir).await.unwrap();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&file_path)
            .await
            .unwrap();

        file.write_all(b"ciaociaociao").await.unwrap();
        file.flush().await.unwrap();
        drop(file);

        let mut delete = Job::try_from(Delete {
            id: request_id,
            file_id,
            force: false,
        })
        .unwrap();
        // NOTE status needs to be in progress for the job to be handled
        delete.status = JobStatus::InProgress;

        storage.handle(delete).await.unwrap();

        assert!(!tokio::fs::try_exists(file_path).await.unwrap());
    }
}
