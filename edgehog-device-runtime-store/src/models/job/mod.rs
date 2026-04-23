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

//! Models for the persistent jobs queue

use diesel::prelude::*;

use crate::conversions::SqlUuid;

use self::job_type::JobType;
use self::status::JobStatus;

pub mod job_type;
pub mod status;

/// A job in the queue
#[derive(Debug, Clone, PartialEq, Eq, Hash, HasQuery, Insertable, Identifiable, AsChangeset)]
#[diesel(table_name = crate::schema::runtime::job_queue)]
#[diesel(primary_key(id, job_type, tag))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Job {
    /// Unique id for the Job Type
    pub id: SqlUuid,
    /// Job type in the queue
    pub job_type: JobType,
    /// Tag to identify the type of data stored
    pub tag: i32,
    /// Status of the job
    pub status: JobStatus,
    /// Version of the serialized data
    pub version: i32,
    /// When the job should be scheduled
    pub schedule_at: Option<i64>,
    /// Serialized additional data for the job
    pub data: Vec<u8>,
}
