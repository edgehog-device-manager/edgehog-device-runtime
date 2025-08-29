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

//! Container device_mapping models.

use std::fmt::Display;

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::exists,
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::SmallInt,
    sqlite::Sqlite,
    Insertable, Queryable, Selectable,
};

use crate::{
    conversions::SqlUuid,
    models::{ExistsFilterById, QueryModel},
    schema::containers::device_mappings,
};

/// Container device mappings with cGroup permissions.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::device_mappings)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct DeviceMapping {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Status of the device mapping.
    pub status: DeviceMappingStatus,
    /// Path on host for the device.
    pub path_on_host: String,
    /// Path in the container for the device.
    pub path_in_container: String,
    /// Permissions on the device.
    ///
    /// For example 'mrw'.
    pub cgroup_permissions: Option<String>,
}

impl QueryModel for DeviceMapping {
    type Table = device_mappings::table;

    type Id = device_mappings::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}
/// Status of a device mapping.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = SmallInt)]
pub enum DeviceMappingStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The device_mapping was acknowledged
    Published = 1,
}

impl Display for DeviceMappingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceMappingStatus::Received => write!(f, "Received"),
            DeviceMappingStatus::Published => write!(f, "Published"),
        }
    }
}

impl From<DeviceMappingStatus> for i16 {
    fn from(value: DeviceMappingStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i16> for DeviceMappingStatus {
    type Error = String;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(DeviceMappingStatus::Received),
            1 => Ok(DeviceMappingStatus::Published),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<SmallInt, Sqlite> for DeviceMappingStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i16::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<SmallInt, Sqlite> for DeviceMappingStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i16::from(*self);

        out.set_value(i32::from(val));

        Ok(IsNull::No)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::DeviceMappingStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            DeviceMappingStatus::Received,
            DeviceMappingStatus::Published,
        ];

        for exp in variants {
            let val = i16::from(exp);

            let status = DeviceMappingStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
