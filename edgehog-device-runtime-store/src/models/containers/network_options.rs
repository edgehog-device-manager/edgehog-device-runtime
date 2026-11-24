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

//! Container network options models.

use crate::{
    conversions::SqlUuid,
    models::{ExistsFilterById, QueryModel},
    schema::containers::{
        network_options, network_options_aliases, network_options_dns_names,
        network_options_driver_opts, network_options_link_local_ips, network_options_links,
    },
};
use bollard::models::{EndpointIpamConfig, EndpointSettings};
use diesel::sql_types::Integer;
use diesel::{
    Insertable, Queryable, Selectable,
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::exists,
    expression::AsExpression,
    prelude::Associations,
    select,
    serialize::{IsNull, ToSql},
    sqlite::Sqlite,
};
use std::fmt::Display;

/// Status of a network options.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum NetworkOptionsStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The network options were acknowledged
    Published = 1,
    /// The network options are missing in the db.
    Missing = 2,
}

impl Display for NetworkOptionsStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkOptionsStatus::Received => write!(f, "Received"),
            NetworkOptionsStatus::Published => write!(f, "Published"),
            NetworkOptionsStatus::Missing => write!(f, "Missing"),
        }
    }
}

impl From<NetworkOptionsStatus> for i32 {
    fn from(value: NetworkOptionsStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for NetworkOptionsStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(NetworkOptionsStatus::Received),
            1 => Ok(NetworkOptionsStatus::Published),
            2 => Ok(NetworkOptionsStatus::Missing),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for NetworkOptionsStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for NetworkOptionsStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// creates an EndpointSettings instance out of a NetworkOptions
pub fn create_endpoint_settings(
    net_options: NetworkOptions,
    NetworkOptionsVecFields {
        network_options_link_local_ips,
        network_options_links,
        network_options_driver_opts,
        network_options_dns_names,
        network_options_aliases,
    }: NetworkOptionsVecFields,
) -> EndpointSettings {
    EndpointSettings {
        ipam_config: Some(EndpointIpamConfig {
            ipv4_address: net_options.ipv4_address,
            ipv6_address: net_options.ipv6_address,
            link_local_ips: if let Some(network_options_link_local_ips) =
                network_options_link_local_ips
            {
                {
                    if network_options_link_local_ips.is_empty() {
                        None
                    } else {
                        Some(
                            network_options_link_local_ips
                                .into_iter()
                                .map(|value| value.ip)
                                .collect(),
                        )
                    }
                }
            } else {
                None
            },
        }),
        links: if let Some(network_options_links) = network_options_links {
            if network_options_links.is_empty() {
                None
            } else {
                Some(
                    network_options_links
                        .into_iter()
                        .map(|value| value.link)
                        .collect(),
                )
            }
        } else {
            None
        },
        aliases: if let Some(network_options_aliases) = network_options_aliases {
            if network_options_aliases.is_empty() {
                None
            } else {
                Some(
                    network_options_aliases
                        .into_iter()
                        .map(|value| value.alias)
                        .collect(),
                )
            }
        } else {
            None
        },
        driver_opts: if let Some(network_options_driver_opts) = network_options_driver_opts {
            if network_options_driver_opts.is_empty() {
                None
            } else {
                Some(
                    network_options_driver_opts
                        .into_iter()
                        .map(|value| (value.option_key, value.option_value))
                        .collect(),
                )
            }
        } else {
            None
        },
        dns_names: if let Some(network_options_dns_names) = network_options_dns_names {
            if network_options_dns_names.is_empty() {
                None
            } else {
                Some(
                    network_options_dns_names
                        .into_iter()
                        .map(|value| value.dns_name)
                        .collect(),
                )
            }
        } else {
            None
        },
        mac_address: net_options.mac_address,
        gw_priority: net_options.gw_priority,
        network_id: Some(net_options.network_id.to_string()),
        endpoint_id: None,
        gateway: net_options.gateway,
        ip_address: None,
        ip_prefix_len: net_options.ip_prefix_len.map(|v| v as i64),
        ipv6_gateway: net_options.ipv6_gateway,
        global_ipv6_address: net_options.global_ipv6_address,
        global_ipv6_prefix_len: net_options.global_ipv6_prefix_len,
    }
}

/// network options vector fields (each stored in its own table)
pub struct NetworkOptionsVecFields {
    /// network options link local ips
    pub network_options_link_local_ips: Option<Vec<NetworkOptionsLinkLocalIp>>,
    /// network options links
    pub network_options_links: Option<Vec<NetworkOptionsLink>>,
    /// network options driver options
    pub network_options_driver_opts: Option<Vec<NetworkOptionsDriverOpt>>,
    /// network options dns names
    pub network_options_dns_names: Option<Vec<NetworkOptionsDnsName>>,
    /// network options aliases
    pub network_options_aliases: Option<Vec<NetworkOptionsAlias>>,
}

/// A list of requests for devices to be sent to device drivers.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::network_options)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct NetworkOptions {
    /// Unique id for the network options request.
    pub id: SqlUuid,
    /// The id of the container to which these network options apply.
    pub container_id: SqlUuid,
    /// The id of the network to connect with the container.
    pub network_id: SqlUuid,
    /// The network options status.
    pub status: NetworkOptionsStatus,
    /// IPv4 address for the endpoint's IPAM configuration.
    pub ipv4_address: Option<String>,
    /// IPv6 address for the endpoint's IPAM configuration.
    pub ipv6_address: Option<String>,
    /// MAC address for the endpoint on this network.
    pub mac_address: Option<String>,
    /// Gateway address for this network.
    pub gw_priority: Option<i64>,
    /// IPv4 address.
    pub gateway: Option<String>,
    /// Set IP address
    pub ip_address: Option<String>,
    /// Mask length of the IPv4 address.
    pub ip_prefix_len: Option<i32>,
    /// IPv6 gateway address.
    pub ipv6_gateway: Option<String>,
    /// Global IPv6 address.
    pub global_ipv6_address: Option<String>,
    /// Mask length of the global IPv6 address.
    pub global_ipv6_prefix_len: Option<i64>,
}

impl QueryModel for NetworkOptions {
    type Table = network_options::table;

    type Id = network_options::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Stores a link-local IP address associated with a network options record.
#[derive(Queryable, Insertable, Associations)]
#[diesel(belongs_to(NetworkOptions, foreign_key = network_options_id))]
#[diesel(table_name = network_options_link_local_ips)]
pub struct NetworkOptionsLinkLocalIp {
    /// ID of the parent network options record.
    pub network_options_id: SqlUuid,
    /// Link-local IP address.
    pub ip: String,
}

/// Stores a link associated with a network options record.
#[derive(Queryable, Insertable, Associations)]
#[diesel(belongs_to(NetworkOptions, foreign_key = network_options_id))]
#[diesel(table_name = network_options_links)]
pub struct NetworkOptionsLink {
    /// ID of the parent network options record.
    pub network_options_id: SqlUuid,
    /// Link value.
    pub link: String,
}

/// Stores an alias associated with a network options record.
#[derive(Queryable, Insertable, Associations)]
#[diesel(belongs_to(NetworkOptions, foreign_key = network_options_id))]
#[diesel(table_name = network_options_aliases)]
pub struct NetworkOptionsAlias {
    /// ID of the parent network options record.
    pub network_options_id: SqlUuid,
    /// Network alias.
    pub alias: String,
}

/// Stores a DNS name associated with a network options record.
#[derive(Queryable, Insertable, Associations)]
#[diesel(belongs_to(NetworkOptions, foreign_key = network_options_id))]
#[diesel(table_name = network_options_dns_names)]
pub struct NetworkOptionsDnsName {
    /// ID of the parent network options record.
    pub network_options_id: SqlUuid,
    /// DNS name.
    pub dns_name: String,
}

/// Stores a driver option key-value pair associated with a network options record.
#[derive(Queryable, Insertable, Associations)]
#[diesel(belongs_to(NetworkOptions, foreign_key = network_options_id))]
#[diesel(table_name = network_options_driver_opts)]
pub struct NetworkOptionsDriverOpt {
    /// ID of the parent network options record.
    pub network_options_id: SqlUuid,
    /// Driver option key.
    pub option_key: String,
    /// Value associated with the driver option key.
    pub option_value: String,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::NetworkOptionsStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            NetworkOptionsStatus::Received,
            NetworkOptionsStatus::Published,
            NetworkOptionsStatus::Missing,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = NetworkOptionsStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
