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

//! Container models.

use std::{fmt::Display, ops::Deref};

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::{exists, Eq, Filter},
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
    Associations, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
};

use crate::{
    conversions::{QuotaValue, SqlUuid, Swappiness},
    models::{
        containers::{
            device_mapping::DeviceMapping, image::Image, network::Network, volume::Volume,
        },
        ExistsFilterById, QueryModel,
    },
    schema::containers::{
        container_missing_device_mappings, container_missing_images, container_missing_networks,
        container_missing_volumes, containers,
    },
};

/// Container configuration.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::containers)]
#[diesel(belongs_to(Image))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Container {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Container id returned by the container engine.
    pub local_id: Option<String>,
    /// Unique id received from Edgehog.
    pub image_id: Option<SqlUuid>,
    /// Status of the volume.
    pub status: ContainerStatus,
    /// Container network mode: none, bridge, ...
    pub network_mode: String,
    /// Hostname for the container
    pub hostname: Option<String>,
    /// Container restart policy
    pub restart_policy: ContainerRestartPolicy,
    /// The length of a CPU period in microseconds.
    pub cpu_period: Option<QuotaValue<-1>>,
    /// Microseconds of CPU time that the container can get in a CPU period.
    pub cpu_quota: Option<QuotaValue<-1>>,
    /// The length of a CPU real-time period in microseconds.
    pub cpu_realtime_period: Option<QuotaValue<-1>>,
    /// The length of a CPU real-time runtime in microseconds.
    pub cpu_realtime_runtime: Option<QuotaValue<-1>>,
    /// Memory limit in bytes.
    pub memory: Option<QuotaValue<-1>>,
    /// Memory soft limit in bytes.
    pub memory_reservation: Option<QuotaValue<-1>>,
    /// Total memory limit (memory + swap).
    pub memory_swap: Option<QuotaValue<-2>>,
    /// Memory swappiness
    pub memory_swappiness: Option<Swappiness>,
    /// Driver that this container uses to mount volumes.
    pub volume_driver: Option<String>,
    /// Mount the container's root filesystem as read only.
    pub read_only_rootfs: bool,
    /// Privileged
    pub privileged: bool,
}

impl QueryModel for Container {
    type Table = containers::table;

    type Id = containers::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Status of a container.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum ContainerStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The container was acknowledged
    Published = 1,
    /// Stopped or exited.
    Stopped = 2,
    /// Up and running.
    Running = 3,
}

impl Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Received => write!(f, "Received"),
            ContainerStatus::Published => write!(f, "Published"),
            ContainerStatus::Stopped => write!(f, "Stopped"),
            ContainerStatus::Running => write!(f, "Running"),
        }
    }
}

impl From<ContainerStatus> for i32 {
    fn from(value: ContainerStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for ContainerStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ContainerStatus::Received),
            1 => Ok(ContainerStatus::Published),
            2 => Ok(ContainerStatus::Stopped),
            3 => Ok(ContainerStatus::Running),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for ContainerStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for ContainerStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// Restart policy of a container.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = Integer)]
pub enum ContainerRestartPolicy {
    /// Empty restart policy
    Empty = 0,
    /// No restart policy
    No = 1,
    /// Always restart the container
    Always = 2,
    /// Unless the container was stopped manually
    UnlessStopped = 3,
    /// On failure
    OnFailure = 4,
}

impl Display for ContainerRestartPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerRestartPolicy::Empty => write!(f, ""),
            ContainerRestartPolicy::No => write!(f, "no"),
            ContainerRestartPolicy::Always => write!(f, "always"),
            ContainerRestartPolicy::UnlessStopped => write!(f, "unless-stopped"),
            ContainerRestartPolicy::OnFailure => write!(f, "on-failure"),
        }
    }
}

impl From<ContainerRestartPolicy> for i32 {
    fn from(value: ContainerRestartPolicy) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for ContainerRestartPolicy {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ContainerRestartPolicy::Empty),
            1 => Ok(ContainerRestartPolicy::No),
            2 => Ok(ContainerRestartPolicy::Always),
            3 => Ok(ContainerRestartPolicy::UnlessStopped),
            4 => Ok(ContainerRestartPolicy::OnFailure),
            _ => Err(format!("unrecognized restart policy value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for ContainerRestartPolicy {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for ContainerRestartPolicy {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_images)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingImage {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`Image`] id
    pub image_id: SqlUuid,
}

type ContainerMissingImageByImage<'a> = Eq<container_missing_images::image_id, &'a SqlUuid>;
type ContainerMissingImageFilterByImage<'a> =
    Filter<container_missing_images::table, ContainerMissingImageByImage<'a>>;

impl ContainerMissingImage {
    /// Returns the filter container_missing_image table by id.
    pub fn by_image(image_id: &SqlUuid) -> ContainerMissingImageByImage<'_> {
        container_missing_images::image_id.eq(image_id)
    }

    /// Returns the filtered container_missing_image table by id.
    pub fn find_by_image(image_id: &SqlUuid) -> ContainerMissingImageFilterByImage<'_> {
        container_missing_images::table.filter(Self::by_image(image_id))
    }
}

/// Networks used by a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_networks)]
#[diesel(belongs_to(Container))]
#[diesel(belongs_to(Network))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerNetwork {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`Network`] id
    pub network_id: SqlUuid,
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_networks)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingNetwork {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`Network`] id
    pub network_id: SqlUuid,
}

type ContainerMissingNetworkByNetwork<'a> = Eq<container_missing_networks::network_id, &'a SqlUuid>;
type ContainerMissingNetworkFilterByNetwork<'a> =
    Filter<container_missing_networks::table, ContainerMissingNetworkByNetwork<'a>>;

impl ContainerMissingNetwork {
    /// Returns the filter container_missing_network table by id.
    pub fn by_network(network_id: &SqlUuid) -> ContainerMissingNetworkByNetwork<'_> {
        container_missing_networks::network_id.eq(network_id)
    }

    /// Returns the filtered container_missing_network table by id.
    pub fn find_by_network(network_id: &SqlUuid) -> ContainerMissingNetworkFilterByNetwork<'_> {
        container_missing_networks::table.filter(Self::by_network(network_id))
    }
}

impl From<ContainerNetwork> for ContainerMissingNetwork {
    fn from(
        ContainerNetwork {
            container_id,
            network_id,
        }: ContainerNetwork,
    ) -> Self {
        Self {
            container_id,
            network_id,
        }
    }
}

/// Volumes used by a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_volumes)]
#[diesel(belongs_to(Container))]
#[diesel(belongs_to(Volume))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerVolume {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`Volume`] id
    pub volume_id: SqlUuid,
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_volumes)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingVolume {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`Volume`] id
    pub volume_id: SqlUuid,
}

type ContainerMissingVolumeByVolume<'a> = Eq<container_missing_volumes::volume_id, &'a SqlUuid>;
type ContainerMissingVolumeFilterByVolume<'a> =
    Filter<container_missing_volumes::table, ContainerMissingVolumeByVolume<'a>>;

impl ContainerMissingVolume {
    /// Returns the filter container_missing_volume table by id.
    pub fn by_volume(volume_id: &SqlUuid) -> ContainerMissingVolumeByVolume<'_> {
        container_missing_volumes::volume_id.eq(volume_id)
    }

    /// Returns the filtered container_missing_volume table by id.
    pub fn find_by_volume(volume_id: &SqlUuid) -> ContainerMissingVolumeFilterByVolume<'_> {
        container_missing_volumes::table.filter(Self::by_volume(volume_id))
    }
}

impl From<ContainerVolume> for ContainerMissingVolume {
    fn from(
        ContainerVolume {
            container_id,
            volume_id,
        }: ContainerVolume,
    ) -> Self {
        Self {
            container_id,
            volume_id,
        }
    }
}

/// Environment variables for a container
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_env)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerEnv {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Environment variable name and optionally a value
    pub value: String,
}

/// Bind mounts for a container
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_binds)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBind {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Environment variable name and optionally a value
    pub value: String,
}

/// Container port bindings
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_port_bindings)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerPortBind {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Container port and optionally protocol
    pub port: String,
    /// Host IP to map the port to
    pub host_ip: Option<String>,
    /// Host port to map the port to
    pub host_port: Option<HostPort>,
}

/// Wrapper to a [`u16`] to be inserted into the database
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = Integer)]
pub struct HostPort(pub u16);

impl Deref for HostPort {
    type Target = u16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromSql<Integer, Sqlite> for HostPort {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        u16::try_from(value).map(HostPort).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for HostPort {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(self.0);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// Extra hosts for a container
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_extra_hosts)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerExtraHost {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Host to add to /etc/hosts inside the container.
    ///
    /// Its in the form of Host:IP
    pub value: String,
}

/// A kernel capabilities to add to the container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_add_capabilities)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerAddCapability {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// A kernel capabilities to add to the container.
    pub value: String,
}

/// A kernel capabilities to drop to the container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_drop_capabilities)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDropCapability {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// A kernel capabilities to drop from the container.
    pub value: String,
}

/// Storage driver options for a container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_storage_options)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerStorageOptions {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Name of the option
    ///
    /// Example: `size`
    pub name: String,
    /// The value of the option
    ///
    /// Example: `120G`
    pub value: Option<String>,
}

/// A map of container directories which should be replaced by tmpfs mounts.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_tmpfs)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerTmpfs {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Path inside the container.
    ///
    /// Example: `/run`
    pub path: String,
    /// Mount options for the tmpfs
    ///
    /// Example: `rw,noexec,nosuid,size=65536k`
    pub options: Option<String>,
}

/// Device Mapping used by a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_device_mappings)]
#[diesel(belongs_to(Container))]
#[diesel(belongs_to(DeviceMapping))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDeviceMapping {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`DeviceMapping`] id
    pub device_mapping_id: SqlUuid,
}

/// Missing device mapping for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_device_mappings)]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingDeviceMapping {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`DeviceMapping`] id
    pub device_mapping_id: SqlUuid,
}

type ContainerMissingDeviceMappingByDeviceMapping<'a> =
    Eq<container_missing_device_mappings::device_mapping_id, &'a SqlUuid>;
type ContainerMissingDeviceMappingFilterByDeviceMapping<'a> = Filter<
    container_missing_device_mappings::table,
    ContainerMissingDeviceMappingByDeviceMapping<'a>,
>;

impl ContainerMissingDeviceMapping {
    /// Returns the filter container_missing_device_mapping table by id.
    pub fn by_device_mapping(
        device_mapping_id: &SqlUuid,
    ) -> ContainerMissingDeviceMappingByDeviceMapping<'_> {
        container_missing_device_mappings::device_mapping_id.eq(device_mapping_id)
    }

    /// Returns the filtered container_missing_device_mapping table by id.
    pub fn find_by_device_mapping(
        device_mapping_id: &SqlUuid,
    ) -> ContainerMissingDeviceMappingFilterByDeviceMapping<'_> {
        container_missing_device_mappings::table.filter(Self::by_device_mapping(device_mapping_id))
    }
}

impl From<ContainerDeviceMapping> for ContainerMissingDeviceMapping {
    fn from(
        ContainerDeviceMapping {
            container_id,
            device_mapping_id,
        }: ContainerDeviceMapping,
    ) -> Self {
        Self {
            container_id,
            device_mapping_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_convert_status() {
        let variants = [
            ContainerStatus::Received,
            ContainerStatus::Published,
            ContainerStatus::Stopped,
            ContainerStatus::Running,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = ContainerStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }

    #[test]
    fn should_convert_restart_policy() {
        let variants = [
            ContainerRestartPolicy::Empty,
            ContainerRestartPolicy::No,
            ContainerRestartPolicy::Always,
            ContainerRestartPolicy::UnlessStopped,
            ContainerRestartPolicy::OnFailure,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = ContainerRestartPolicy::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
