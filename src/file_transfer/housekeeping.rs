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

//! Housekeeping task for the FileTransfer

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use astarte_device_sdk::Client;
use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::{Context, OptionExt, bail};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument};
use uuid::Uuid;

use crate::{file_transfer::interface::file::StoredFile, jobs::Queue};

/// Task that handles scheduled task for the file transfer
pub struct StorageTask<C> {
    queue: Queue,
    notify: Arc<Notify>,
    device: C,
}

impl<C> StorageTask<C> {
    pub(crate) fn new(queue: Queue, notify: Arc<Notify>, device: C) -> Self {
        Self {
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

        if let Some(timestamp) = self.queue.next_schedule(JobType::FileStorage).await? {
            let Some(instant) = timestamp.wait_until()? else {
                return Ok(true);
            };

            let sleep = tokio::time::sleep_until(instant.into());

            tokio::select! {
                () = sleep => {
                    return Ok(true);
                }
                () = self.notify.notified() => {
                    return Ok(true);
                }
                () = cancel.cancelled() => {
                    return Ok(false);
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
            && let Some(job) = self.queue.next_scheduled_job(JobType::FileStorage).await?
        {
            if let Err(error) = self.handle(&job).await {
                error!(%error,"couldn't handle storage job");
            }
        }

        Ok(())
    }

    #[instrument(skip_all, fields(id = %job.id))]
    async fn handle(&mut self, job: &Job) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        let job = CleanUp::try_from(job)?;

        if job.file_path.is_dir() {
            tokio::fs::remove_dir_all(&job.file_path)
                .await
                .wrap_err("couldn't remove directory")?;
        } else {
            tokio::fs::remove_file(&job.file_path)
                .await
                .wrap_err("couldn't remove file")?;
        }

        StoredFile::deleted(job.id, &mut self.device).await;

        // Just delete the job since we don't need to do anything for completion
        self.queue
            .delete(&job.id, JobType::FileStorage, StorageJobTag::CleanUp.into())
            .await?;

        Ok(())
    }
}

/// File cleanup job.
#[derive(Debug, minicbor::Encode, minicbor::Decode)]
pub(crate) struct CleanUp<'a> {
    #[cbor(skip)]
    pub(crate) id: Uuid,
    #[cbor(skip)]
    pub(crate) schedule_at: i64,
    #[n(0)]
    pub(crate) file_path: Cow<'a, Path>,
}

impl<'a> CleanUp<'a> {
    const SERIALIZED_VERSION: i32 = 0;
}

impl<'a> TryFrom<&'a Job> for CleanUp<'a> {
    type Error = eyre::Report;

    fn try_from(value: &'a Job) -> Result<Self, Self::Error> {
        let Job {
            id,
            job_type,
            status,
            version,
            tag,
            schedule_at,
            data,
        } = value;

        debug_assert_eq!(*job_type, JobType::FileStorage);
        debug_assert_eq!(*status, JobStatus::InProgress);
        debug_assert_eq!(*tag, i32::from(StorageJobTag::CleanUp));
        debug_assert_eq!(*version, Self::SERIALIZED_VERSION);

        let mut this: Self = minicbor::decode(data)?;

        this.id = **id;
        this.schedule_at = schedule_at.ok_or_eyre("missing schedule_at field")?;

        Ok(this)
    }
}

impl<'a> TryFrom<CleanUp<'a>> for Job {
    type Error = eyre::Report;

    fn try_from(value: CleanUp) -> Result<Self, Self::Error> {
        let data = minicbor::to_vec(&value)?;

        Ok(Job {
            id: SqlUuid::from(value.id),
            job_type: JobType::FileStorage,
            status: JobStatus::Pending,
            version: CleanUp::SERIALIZED_VERSION,
            tag: StorageJobTag::CleanUp.into(),
            schedule_at: Some(value.schedule_at),
            data,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum StorageJobTag {
    CleanUp = 0,
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
            _ => bail!("unrecognize file transfer job tag {value}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::with_insta;
    use rstest::{Context, rstest};

    use super::*;

    #[rstest]
    #[case(StorageJobTag::CleanUp)]
    fn job_tag_roundtrip(#[context] ctx: Context, #[case] value: StorageJobTag) {
        let buf = i32::from(value);

        let res = StorageJobTag::try_from(buf).unwrap();

        assert_eq!(res, value);

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, format!("{value:?} = {}", buf));
        });
    }
}
