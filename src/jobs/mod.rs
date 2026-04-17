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

//! Job queue

use diesel::{delete, insert_into, prelude::*, update};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::Handle;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use edgehog_store::schema::runtime::job_queue;
use eyre::Context;
use tracing::{debug, instrument};
use uuid::Uuid;

pub(crate) mod derive;

/// Persistent Job Queue
#[derive(Debug, Clone)]
pub struct Queue {
    db: Handle,
}

impl Queue {
    pub fn new(db: Handle) -> Self {
        Self { db }
    }

    #[instrument(skip(self))]
    pub async fn init(&self) -> eyre::Result<()> {
        self.db
            .for_write(move |write| {
                let row_changed = update(job_queue::table)
                    .filter(job_queue::status.eq(JobStatus::InProgress))
                    .set(job_queue::status.eq(JobStatus::Pending))
                    .execute(write)?;

                debug!(row_changed, "job status reset");

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[cfg_attr(not(test), expect(unused))]
    #[instrument(skip_all)]
    pub async fn insert<J>(&self, job: J) -> eyre::Result<()>
    where
        J: TryInto<Job, Error = eyre::Report>,
    {
        let job = job.try_into()?;

        self.insert_job(job).await
    }

    #[instrument(skip(self))]
    pub async fn next<J>(&self, job_type: JobType) -> eyre::Result<Option<J>>
    where
        Job: TryInto<J, Error = eyre::Report>,
    {
        self.next_job(job_type)
            .await
            .and_then(|job| job.map(TryInto::try_into).transpose())
    }

    #[instrument(skip(self))]
    pub async fn insert_job(&self, job: Job) -> eyre::Result<()> {
        self.db
            .for_write(move |write| {
                let row_changed = insert_into(job_queue::table)
                    .values(&job)
                    .on_conflict_do_nothing()
                    .execute(write)?;

                debug!(row_changed, "job inserted");

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn next_job(&self, job_type: JobType) -> eyre::Result<Option<Job>> {
        let job = self
            .db
            .for_write(move |write| {
                let Some(mut job) = Job::query()
                    .filter(job_queue::job_type.eq(job_type))
                    .filter(job_queue::status.eq(JobStatus::Pending))
                    .order_by(job_queue::created_at.asc())
                    .first(write)
                    .optional()?
                else {
                    return Ok(None);
                };

                job.status = JobStatus::InProgress;

                update(job_queue::table).set(&job).execute(write)?;

                Ok(Some(job))
            })
            .await?;

        Ok(job)
    }

    #[instrument(skip_all)]
    pub async fn update(&self, id: &Uuid, tag: i32, status: JobStatus) -> eyre::Result<()> {
        let id = SqlUuid::new(*id);

        self.db
            .for_write(move |write| {
                let updated_rows = update(job_queue::table)
                    .filter(job_queue::id.eq(id))
                    .filter(job_queue::tag.eq(tag))
                    .set(job_queue::status.eq(status))
                    .execute(write)?;

                debug!(updated_rows, "job updated");

                Ok(())
            })
            .await
            .wrap_err("couldn't delete job")
    }

    #[instrument(skip_all)]
    pub async fn delete(&self, id: &Uuid, tag: i32) -> eyre::Result<()> {
        let id = SqlUuid::new(*id);

        self.db
            .for_write(move |write| {
                let deleted_rows = delete(job_queue::table)
                    .filter(job_queue::id.eq(id))
                    .filter(job_queue::tag.eq(tag))
                    .execute(write)?;

                debug!(deleted_rows, "job deleted");

                Ok(())
            })
            .await
            .wrap_err("couldn't delete job")
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use edgehog_store::conversions::SqlUuid;
    use edgehog_store::db::HandleError;
    use edgehog_store::models::job::status::JobStatus;
    use pretty_assertions::assert_eq;
    use tempdir::TempDir;
    use uuid::Uuid;

    use super::*;

    impl Queue {
        pub(crate) async fn fetch_job(&self, id: &Uuid) -> Option<Job> {
            let id = *id;

            self.db
                .for_read(move |read| {
                    Job::query()
                        .filter(job_queue::id.eq(SqlUuid::new(id)))
                        .first(read)
                        .optional()
                        .map_err(HandleError::from)
                })
                .await
                .unwrap()
        }
    }

    async fn queue(prefix: &str) -> (Queue, TempDir) {
        let dir = TempDir::new(prefix).unwrap();

        let db = Handle::open(dir.path().join("database.db")).await.unwrap();

        (Queue::new(db), dir)
    }

    #[tokio::test]
    async fn reset() {
        let (queue, _dir) = queue("insert").await;

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(exp.clone()).await.unwrap();

        exp.status = JobStatus::InProgress;

        let job = queue
            .next_job(JobType::FileTransfer)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(job, exp);
    }

    #[tokio::test]
    async fn insert_and_next() {
        let (queue, _dir) = queue("insert_and_next").await;

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(exp.clone()).await.unwrap();

        exp.status = JobStatus::InProgress;

        let job = queue
            .next_job(JobType::FileTransfer)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(job, exp);
    }

    #[tokio::test]
    async fn next_pending() {
        let (queue, _dir) = queue("next_pending").await;

        let in_progress = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::InProgress,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(in_progress).await.unwrap();

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(exp.clone()).await.unwrap();

        exp.status = JobStatus::InProgress;

        let job = queue
            .next_job(JobType::FileTransfer)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(job, exp);
    }

    #[tokio::test]
    async fn update_job() {
        let (queue, _dir) = queue("update_job").await;

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::InProgress,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(exp.clone()).await.unwrap();

        queue.update(&exp.id, 0, JobStatus::Done).await.unwrap();

        exp.status = JobStatus::Done;

        let job = queue.fetch_job(&exp.id).await.unwrap();

        assert_eq!(job, exp);
    }

    #[tokio::test]
    async fn delete_job() {
        let (queue, _dir) = queue("update_job").await;

        let exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::InProgress,
            version: 0,
            tag: 0,
            data: vec![42],
        };

        queue.insert_job(exp.clone()).await.unwrap();

        queue.delete(&exp.id, 0).await.unwrap();

        let job = queue.fetch_job(&exp.id).await;

        assert!(job.is_none());
    }
}
