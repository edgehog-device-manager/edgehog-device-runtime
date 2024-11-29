// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Container network models.

use std::fmt::Display;

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::{exists, BareSelect, Eq, Filter},
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
    Associations, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
};

use crate::{conversions::SqlUuid, schema::containers::networks};

/// Container network with driver configuration.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::networks)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct Network {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Network id returned by the container engine.
    pub local_id: Option<String>,
    /// Status of the network.
    pub status: NetworkStatus,
    /// Driver to use for the network.
    pub driver: String,
    /// Mark the network as internal.
    pub internal: bool,
    /// Enable ipv6 for the network
    pub enable_ipv6: bool,
}

type NetworkById<'a> = Eq<networks::id, &'a SqlUuid>;
type NetworkFilterById<'a> = Filter<networks::table, NetworkById<'a>>;
type NetworkExists<'a> = BareSelect<exists<Filter<networks::table, NetworkById<'a>>>>;

impl Network {
    /// Returns the filter network table by id.
    pub fn by_id(id: &SqlUuid) -> NetworkById<'_> {
        networks::id.eq(id)
    }

    /// Returns the filtered network table by id.
    pub fn find_id(id: &SqlUuid) -> NetworkFilterById<'_> {
        networks::table.filter(Self::by_id(id))
    }

    /// Returns the network exists query.
    pub fn exists(id: &SqlUuid) -> NetworkExists<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Container network with driver configuration.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::network_driver_opts)]
#[diesel(belongs_to(Network))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct NetworkDriverOpts {
    /// Id of the network.
    pub network_id: SqlUuid,
    /// Name of the driver option
    pub name: String,
    /// Value of the driver option
    pub value: Option<String>,
}

/// Status of a network.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum NetworkStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The network was acknowledged
    Published = 1,
    /// Created on the runtime.
    Created = 2,
}

impl Display for NetworkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkStatus::Received => write!(f, "Received"),
            NetworkStatus::Published => write!(f, "Published"),
            NetworkStatus::Created => write!(f, "Created"),
        }
    }
}

impl From<NetworkStatus> for i32 {
    fn from(value: NetworkStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for NetworkStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(NetworkStatus::Received),
            1 => Ok(NetworkStatus::Published),
            2 => Ok(NetworkStatus::Created),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for NetworkStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for NetworkStatus {
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
    use super::NetworkStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            NetworkStatus::Received,
            NetworkStatus::Published,
            NetworkStatus::Created,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = NetworkStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
