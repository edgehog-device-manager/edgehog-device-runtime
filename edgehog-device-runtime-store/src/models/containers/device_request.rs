// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

//! Container device_request models.

use std::fmt::Display;

use diesel::{
    Insertable, Queryable, Selectable,
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::exists,
    expression::AsExpression,
    prelude::Associations,
    select,
    serialize::{IsNull, ToSql},
    sql_types::SmallInt,
    sqlite::Sqlite,
};

use crate::{
    conversions::SqlUuid,
    models::{ExistsFilterById, QueryModel},
    schema::containers::device_requests,
};

/// A list of requests for devices to be sent to device drivers.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::device_requests)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct DeviceRequest {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Status of the device request.
    pub status: DeviceRequestStatus,
    /// The name of the device driver to use for this request.
    ///
    /// Note that if this is specified the capabilities are ignored when selecting a device driver.
    pub driver: Option<String>,
    /// Path in the container for the device.
    pub count: i64,
}

impl QueryModel for DeviceRequest {
    type Table = device_requests::table;

    type Id = device_requests::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Device request device id.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::device_requests_device_ids)]
#[diesel(belongs_to(DeviceRequest))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct DeviceRequestDeviceId {
    /// Id of the device request.
    pub device_request_id: SqlUuid,
    /// Id of the device
    pub device_id: String,
}

/// Device request capabilities.
///
/// A list of capabilities; an OR list of AND lists of capabilities.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::device_requests_capabilities)]
#[diesel(belongs_to(DeviceRequest))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct DeviceRequestCapability {
    /// Id of the device request.
    pub device_request_id: SqlUuid,
    /// Index of the OR list of capabilities.
    pub idx: i32,
    /// Capability value.
    pub capability: String,
}

/// Driver-specific options, specified as a key/value pairs.
///
/// These options are passed directly to the driver.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::device_requests_options)]
#[diesel(belongs_to(DeviceRequest))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct DeviceRequestOption {
    /// Id of the device request.
    pub device_request_id: SqlUuid,
    /// Name of the option
    pub name: String,
    /// Option value.
    pub value: String,
}

/// Status of a device request.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = SmallInt)]
pub enum DeviceRequestStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The device_request was acknowledged
    Published = 1,
}

impl Display for DeviceRequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceRequestStatus::Received => write!(f, "Received"),
            DeviceRequestStatus::Published => write!(f, "Published"),
        }
    }
}

impl From<DeviceRequestStatus> for i16 {
    fn from(value: DeviceRequestStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i16> for DeviceRequestStatus {
    type Error = String;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(DeviceRequestStatus::Received),
            1 => Ok(DeviceRequestStatus::Published),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<SmallInt, Sqlite> for DeviceRequestStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i16::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<SmallInt, Sqlite> for DeviceRequestStatus {
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

    use super::DeviceRequestStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            DeviceRequestStatus::Received,
            DeviceRequestStatus::Published,
        ];

        for exp in variants {
            let val = i16::from(exp);

            let status = DeviceRequestStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
