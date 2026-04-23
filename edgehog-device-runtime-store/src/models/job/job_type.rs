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

//! Status of a job.

use diesel::backend::Backend;
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::serialize::{IsNull, ToSql};
use diesel::sql_types::Integer;
use diesel::sqlite::Sqlite;

/// Status of the job
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
#[repr(u8)]
pub enum JobType {
    /// File transfer request between the Server and Device
    #[default]
    FileTransfer = 0,
    /// File storage task
    FileStorage = 1,
}

impl From<JobType> for i32 {
    fn from(value: JobType) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for JobType {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, String> {
        match value {
            0 => Ok(JobType::FileTransfer),
            1 => Ok(JobType::FileStorage),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl<DB> FromSql<Integer, DB> for JobType
where
    DB: Backend,
    i32: FromSql<Integer, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for JobType {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

#[cfg(test)]
mod tests {
    use rstest::{Context, rstest};

    use crate::tests::with_insta;

    use super::*;

    #[rstest]
    #[case(JobType::FileTransfer)]
    #[case(JobType::FileStorage)]
    fn job_type_roundtrip(#[context] ctx: Context, #[case] value: JobType) {
        let i = i32::from(value);

        let res = JobType::try_from(i).unwrap();

        assert_eq!(res, value);

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, format!("{value:?} = {i}"));
        });
    }
}
