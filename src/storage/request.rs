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

use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::Context;
use eyre::OptionExt;
use uuid::Uuid;

use crate::jobs::derive;
use crate::storage::interface::ActionType;

use super::StorageJobTag;
use super::interface;

/// File delete request.
#[derive(Debug, minicbor::Encode, minicbor::Decode)]
pub(crate) struct Delete {
    #[cbor(skip)]
    pub(crate) id: Uuid,
    #[cbor(n(0), with = "derive::uuid")]
    pub(crate) file_id: Uuid,
    #[n(1)]
    pub(crate) force: bool,
}

impl Delete {
    const SERIALIZED_VERSION: i32 = 0;

    pub(crate) fn id(&self) -> FileStorageId {
        FileStorageId {
            id: self.id,
            ty: ActionType::Delete,
        }
    }
}

impl TryFrom<&interface::DeleteFile> for Delete {
    type Error = eyre::Report;

    fn try_from(value: &interface::DeleteFile) -> Result<Self, Self::Error> {
        let interface::DeleteFile { id, file_id, force } = value;

        let id = Uuid::parse_str(id).wrap_err("could not parse id")?;
        let file_id = Uuid::parse_str(file_id).wrap_err("could not parse file_id")?;

        Ok(Self {
            id,
            file_id,
            force: *force,
        })
    }
}

impl TryFrom<&Job> for Delete {
    type Error = eyre::Report;

    fn try_from(value: &Job) -> Result<Self, Self::Error> {
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
        debug_assert_eq!(*tag, i32::from(StorageJobTag::Delete));
        debug_assert_eq!(*version, Self::SERIALIZED_VERSION);
        debug_assert_eq!(*schedule_at, None);

        let mut this: Self = minicbor::decode(data)?;

        this.id = **id;

        Ok(this)
    }
}

impl TryFrom<Delete> for Job {
    type Error = eyre::Report;

    fn try_from(value: Delete) -> Result<Self, Self::Error> {
        let data = minicbor::to_vec(&value).wrap_err("couldn't encode download request")?;

        Ok(Job {
            id: SqlUuid::new(value.id),
            job_type: JobType::FileStorage,
            status: JobStatus::default(),
            version: Delete::SERIALIZED_VERSION,
            tag: StorageJobTag::Delete.into(),
            data,
            schedule_at: None,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct FileStorageId {
    pub(crate) id: Uuid,
    pub(crate) ty: ActionType,
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
