// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Container volume models.

use std::fmt::Display;

use diesel::{
    Insertable, Queryable, Selectable,
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::exists,
    expression::AsExpression,
    prelude::Identifiable,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
};

use crate::{
    conversions::SqlUuid,
    models::{ExistsFilterById, QueryModel},
    schema::containers::volumes,
};

/// Container volume with driver configuration.
#[derive(Debug, Clone, Insertable, Identifiable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::volumes)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Volume {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Status of the volume.
    pub status: VolumeStatus,
    /// Driver to use for the volume.
    pub driver: String,
}

impl QueryModel for Volume {
    type Table = volumes::table;

    type Id = volumes::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Container volume with driver configuration.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::volume_driver_opts)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct VolumeDriverOpts {
    /// Id of the volume.
    pub volume_id: SqlUuid,
    /// Name of the driver option
    pub name: String,
    /// Value of the driver option
    pub value: String,
}

/// Status of a volume.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum VolumeStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The volume was acknowledged
    Published = 1,
    /// Created on the runtime.
    Created = 2,
}

impl Display for VolumeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VolumeStatus::Received => write!(f, "Received"),
            VolumeStatus::Published => write!(f, "Published"),
            VolumeStatus::Created => write!(f, "Created"),
        }
    }
}

impl From<VolumeStatus> for i32 {
    fn from(value: VolumeStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for VolumeStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(VolumeStatus::Received),
            1 => Ok(VolumeStatus::Published),
            2 => Ok(VolumeStatus::Created),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for VolumeStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for VolumeStatus {
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
    use super::VolumeStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            VolumeStatus::Received,
            VolumeStatus::Published,
            VolumeStatus::Created,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = VolumeStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
