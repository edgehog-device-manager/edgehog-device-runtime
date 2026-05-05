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

use diesel::dsl::{exists, select};
use diesel::{delete, insert_into, prelude::*, update};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::{Handle, HandleError};
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use edgehog_store::schema::runtime::job_queue;
use eyre::Context;
use tracing::{debug, instrument, trace};
use uuid::Uuid;

use self::timestamp::Unix;

pub(crate) mod derive;
pub(crate) mod timestamp;

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

    #[instrument(skip_all)]
    pub async fn exists(&self, id: &Uuid, job_type: JobType, tag: i32) -> eyre::Result<bool> {
        let id = SqlUuid::new(*id);

        self.db
            .for_read(move |write| {
                let exists: bool = select(exists(
                    job_queue::table
                        .filter(job_queue::id.eq(id))
                        .filter(job_queue::job_type.eq(job_type))
                        .filter(job_queue::tag.eq(tag)),
                ))
                .get_result(write)?;

                Ok(exists)
            })
            .await
            .wrap_err("couldn't delete job")
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
    pub async fn insert_if_missing(&self, job: Job) -> eyre::Result<bool> {
        let was_inserted = self
            .db
            .for_write(move |write| {
                let exists: bool = select(exists(
                    job_queue::table
                        .filter(job_queue::id.eq(&job.id))
                        .filter(job_queue::job_type.eq(job.job_type))
                        .filter(job_queue::tag.eq(job.tag)),
                ))
                .get_result(write)?;

                if !exists {
                    let row_changed = insert_into(job_queue::table).values(&job).execute(write)?;

                    debug!(row_changed, "job inserted");

                    Ok(true)
                } else {
                    debug!("job already exists");

                    Ok(false)
                }
            })
            .await?;

        Ok(was_inserted)
    }

    #[instrument(skip(self))]
    pub async fn next_job(&self, job_type: JobType) -> eyre::Result<Option<Job>> {
        let job = self
            .db
            .for_write(move |write| {
                let optional = Job::query()
                    .filter(job_queue::job_type.eq(job_type))
                    .filter(job_queue::status.eq(JobStatus::Pending))
                    .order_by(job_queue::created_at.asc())
                    .first(write)
                    .optional()?;

                let Some(mut job) = optional else {
                    return Ok(None);
                };

                job.status = JobStatus::InProgress;

                let row_changed = update(&job)
                    .set(job_queue::status.eq(JobStatus::InProgress))
                    .execute(write)?;

                debug_assert_eq!(row_changed, 1);

                trace!(row_changed, id = %job.id, "updated job to in progress");

                Ok(Some(job))
            })
            .await?;

        Ok(job)
    }

    /// Returns the next instant to sleep until to schedule the job
    ///
    /// It will return an instant at the nth second.
    #[instrument(skip(self))]
    pub async fn next_schedule(&self, job_type: JobType) -> eyre::Result<Option<Unix>> {
        let first = self
            .db
            .for_read(move |read| {
                job_queue::table
                    .select(job_queue::schedule_at.assume_not_null())
                    .filter(job_queue::schedule_at.is_not_null())
                    .filter(job_queue::job_type.eq(job_type))
                    .filter(job_queue::status.eq(JobStatus::Pending))
                    .order_by(job_queue::schedule_at.asc())
                    .first::<i64>(read)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await?;

        first.map(Unix::try_from).transpose()
    }

    #[instrument(skip(self))]
    pub async fn next_scheduled_job(&self, job_type: JobType) -> eyre::Result<Option<Job>> {
        let now = Unix::now()?;

        let job = self
            .db
            .for_write(move |write| {
                let optional = Job::query()
                    .filter(job_queue::schedule_at.is_not_null())
                    .filter(job_queue::schedule_at.le(now.as_i64()))
                    .filter(job_queue::job_type.eq(job_type))
                    .filter(job_queue::status.eq(JobStatus::Pending))
                    .order_by(job_queue::schedule_at.asc())
                    .first(write)
                    .optional()?;

                let Some(mut job) = optional else {
                    return Ok(None);
                };

                job.status = JobStatus::InProgress;

                let row_changed = update(&job)
                    .set(job_queue::status.eq(JobStatus::InProgress))
                    .execute(write)?;

                debug_assert_eq!(row_changed, 1);

                Ok(Some(job))
            })
            .await?;

        Ok(job)
    }

    #[instrument(skip_all)]
    pub async fn update(
        &self,
        id: &Uuid,
        job_type: JobType,
        tag: i32,
        status: JobStatus,
    ) -> eyre::Result<()> {
        let id = SqlUuid::new(*id);

        self.db
            .for_write(move |write| {
                let updated_rows = update(job_queue::table)
                    .filter(job_queue::id.eq(id))
                    .filter(job_queue::job_type.eq(job_type))
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
    pub async fn delete(&self, id: &Uuid, job_type: JobType, tag: i32) -> eyre::Result<()> {
        let id = SqlUuid::new(*id);

        self.db
            .for_write(move |write| {
                let deleted_rows = delete(job_queue::table)
                    .filter(job_queue::id.eq(id))
                    .filter(job_queue::job_type.eq(job_type))
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
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use edgehog_store::conversions::SqlUuid;
    use edgehog_store::db::HandleError;
    use edgehog_store::models::job::status::JobStatus;
    use pretty_assertions::assert_eq;
    use rstest::{fixture, rstest};
    use tempdir::TempDir;
    use uuid::Uuid;

    use crate::file_transfer::request::TransferJobTag;

    use super::*;

    impl Queue {
        pub(crate) async fn fetch_file_transfer(
            &self,
            id: &Uuid,
            tag: TransferJobTag,
        ) -> Option<Job> {
            self.fetch_job(id, JobType::FileTransfer, tag.into()).await
        }

        pub(crate) async fn fetch_job(
            &self,
            id: &Uuid,
            job_type: JobType,
            tag: i32,
        ) -> Option<Job> {
            let id = *id;

            self.db
                .for_read(move |read| {
                    Job::query()
                        .filter(job_queue::id.eq(SqlUuid::new(id)))
                        .filter(job_queue::job_type.eq(job_type))
                        .filter(job_queue::tag.eq(tag))
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

    #[fixture]
    fn simple_job() -> Job {
        Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
            schedule_at: None,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn reset(mut simple_job: Job) {
        let (queue, _dir) = queue("insert").await;

        queue.insert_job(simple_job.clone()).await.unwrap();

        simple_job.status = JobStatus::InProgress;

        let job = queue
            .next_job(JobType::FileTransfer)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(job, simple_job);
    }

    #[rstest]
    #[tokio::test]
    async fn insert_and_next(mut simple_job: Job) {
        let (queue, _dir) = queue("insert_and_next").await;

        queue.insert_job(simple_job.clone()).await.unwrap();

        simple_job.status = JobStatus::InProgress;

        let job = queue
            .next_job(JobType::FileTransfer)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(job, simple_job);
    }

    #[rstest]
    #[tokio::test]
    async fn insert_and_exists(mut simple_job: Job) {
        let (queue, _dir) = queue("insert_and_next").await;

        queue.insert_job(simple_job.clone()).await.unwrap();

        simple_job.status = JobStatus::InProgress;

        let job_exists = queue
            .exists(&simple_job.id, simple_job.job_type, simple_job.tag)
            .await
            .unwrap();

        assert!(job_exists);
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
            schedule_at: None,
        };

        queue.insert_job(in_progress).await.unwrap();

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileTransfer,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
            schedule_at: None,
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
            schedule_at: None,
        };

        queue.insert_job(exp.clone()).await.unwrap();

        queue
            .update(&exp.id, JobType::FileTransfer, 0, JobStatus::Done)
            .await
            .unwrap();

        exp.status = JobStatus::Done;

        let job = queue
            .fetch_job(&exp.id, JobType::FileTransfer, 0)
            .await
            .unwrap();

        assert_eq!(job, exp);
    }

    #[rstest]
    #[tokio::test]
    async fn delete_job(simple_job: Job) {
        let (queue, _dir) = queue("update_job").await;

        queue.insert_job(simple_job.clone()).await.unwrap();

        queue
            .delete(&simple_job.id, JobType::FileTransfer, 0)
            .await
            .unwrap();

        let job = queue
            .fetch_job(&simple_job.id, JobType::FileTransfer, 0)
            .await;

        assert!(job.is_none());
    }

    #[tokio::test]
    async fn fetch_next_schedule() {
        let (queue, _dir) = queue("next_schedule").await;

        let sched = SystemTime::now()
            .checked_add(Duration::from_secs(120))
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sched_sig = sched.try_into().unwrap();

        let exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileStorage,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
            schedule_at: Some(sched_sig),
        };

        queue.insert_job(exp.clone()).await.unwrap();

        let instant = queue
            .next_schedule(JobType::FileStorage)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(u64::try_from(instant.as_i64()).unwrap(), sched)
    }

    #[tokio::test]
    async fn fetch_next_schedule_job() {
        let (queue, _dir) = queue("next_schedule").await;

        // in the past
        let sched = SystemTime::now()
            .checked_sub(Duration::from_secs(120))
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let sched_sig = sched.try_into().unwrap();

        let mut exp = Job {
            id: SqlUuid::new(Uuid::new_v4()),
            job_type: JobType::FileStorage,
            status: JobStatus::Pending,
            version: 0,
            tag: 0,
            data: vec![42],
            schedule_at: Some(sched_sig),
        };

        queue.insert_job(exp.clone()).await.unwrap();

        let res = queue
            .next_scheduled_job(JobType::FileStorage)
            .await
            .unwrap()
            .unwrap();

        exp.status = JobStatus::InProgress;

        assert_eq!(res, exp);
    }
}
