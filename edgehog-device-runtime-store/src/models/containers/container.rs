// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

//! Container models.

use std::{borrow::Cow, fmt::Display, ops::Deref, str::FromStr};

use diesel::{
    Associations, ExpressionMethods, HasQuery, Insertable, QueryDsl, Queryable, Selectable,
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::{Eq, Filter, exists},
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::{Integer, SmallInt},
    sqlite::Sqlite,
};

use crate::conversions::SqlUuid;
use crate::models::containers::device_mapping::DeviceMapping;
use crate::models::containers::device_request::DeviceRequest;
use crate::models::containers::image::Image;
use crate::models::containers::network::Network;
use crate::models::containers::volume::Volume;
use crate::models::{ExistsFilterById, QueryModel};
use crate::schema::containers::{
    container_missing_device_mappings, container_missing_device_requests, container_missing_images,
    container_missing_networks, container_missing_volumes, containers,
};

/// Container configuration.
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::containers)]
#[diesel(belongs_to(Image))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Container<'a> {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Container id returned by the container engine.
    pub local_id: Option<Cow<'a, str>>,
    /// Unique id received from Edgehog.
    pub image_id: Option<SqlUuid>,
    /// Status of the volume.
    pub status: ContainerStatus,
    /// Container network mode: none, bridge, ...
    pub network_mode: Option<Cow<'a, str>>,
    /// Hostname for the container
    pub hostname: Option<Cow<'a, str>>,
    /// Container restart policy
    pub restart_policy: Option<ContainerRestartPolicy>,
    /// If on-failure is used, the number of times to retry before giving up.
    pub restart_policy_maximum_retry_count: Option<i32>,
    /// The length of a CPU period in microseconds.
    pub cpu_period: Option<i64>,
    /// Microseconds of CPU time that the container can get in a CPU period.
    pub cpu_quota: Option<i64>,
    /// The length of a CPU real-time period in microseconds.
    pub cpu_realtime_period: Option<i64>,
    /// The length of a CPU real-time runtime in microseconds.
    pub cpu_realtime_runtime: Option<i64>,
    /// An integer value representing this container's relative CPU weight versus other containers.
    pub cpu_shares: Option<i32>,
    /// Memory limit in bytes.
    pub memory: Option<i64>,
    /// Memory soft limit in bytes.
    pub memory_reservation: Option<i64>,
    /// Total memory limit (memory + swap).
    pub memory_swap: Option<i64>,
    /// Memory swappiness
    pub memory_swappiness: Option<i16>,
    /// Driver that this container uses to mount volumes.
    pub volume_driver: Option<Cow<'a, str>>,
    /// Mount the container's root filesystem as read only.
    pub read_only_rootfs: Option<bool>,
    /// Privileged
    pub privileged: Option<bool>,
    /// Domain name for the container
    pub domainname: Option<Cow<'a, str>>,
    /// User and group running in the container
    pub user: Option<Cow<'a, str>>,
    /// The time to wait between checks in nanoseconds.
    pub health_check_interval: Option<i64>,
    /// The time to wait before considering the check to have hung in nanoseconds.
    pub health_check_timeout: Option<i64>,
    /// The number of consecutive failures needed to consider a container as unhealthy
    pub health_check_retries: Option<i32>,
    /// Start period for the container to initialize before starting health-retries countdown in nanoseconds.
    ///
    /// It should be 0 or at least 1000000 (1 ms). 0 means inherit.
    pub health_check_start_period: Option<i64>,
    /// The time to wait between checks in nanoseconds during the start period.
    ///
    /// It should be 0 or at least 1000000 (1 ms). 0 means inherit.
    pub health_check_start_interval: Option<i64>,
    /// The working directory for commands to run in.
    pub working_dir: Option<Cow<'a, str>>,
    /// Disable networking for the container.
    pub network_disabled: Option<bool>,
    /// Disable networking for the container.
    pub stop_signal: Option<Cow<'a, str>>,
    /// Timeout to stop a container in seconds.
    pub stop_timeout: Option<i32>,
    /// Automatically remove the container when the container's process exits.
    ///
    /// This has no effect if RestartPolicy is set.
    pub auto_remove: Option<bool>,
    /// cgroup namespace mode for the container.
    pub cgroupns_mode: Option<CgroupnsMode>,
    /// IPC sharing mode for the container.
    pub ipc_mode: Option<Cow<'a, str>>,
    /// An integer value containing the score given to the container in order to tune OOM killer preferences.
    pub oom_score_adj: Option<i32>,
    /// Sets the usernamespace mode for the container when usernamespace remapping option is enabled.
    pub userns_mode: Option<Cow<'a, str>>,
    /// Size of /dev/shm in bytes.
    pub shm_size: Option<i64>,
    /// Name of the logging driver used for the container or "none" if logging is disabled.
    pub log_type: Option<Cow<'a, str>>,
    /// Block IO weight (relative weight).
    pub blkio_weight: Option<i16>,
    /// Set the PID (Process) Namespace mode for the container.
    pub pid_mode: Option<Cow<'a, str>>,
    /// Runtime to use with this container.
    pub runtime: Option<Cow<'a, str>>,
}

impl<'c> QueryModel for Container<'c> {
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

impl FromStr for ContainerRestartPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "" => Ok(ContainerRestartPolicy::Empty),
            "no" => Ok(ContainerRestartPolicy::No),
            "always" => Ok(ContainerRestartPolicy::Always),
            "unless-stopped" => Ok(ContainerRestartPolicy::UnlessStopped),
            "on-failure" => Ok(ContainerRestartPolicy::OnFailure),
            _ => Err(format!("couldn't parse container restart policy {s}")),
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

/// CGruop name-space mode
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = SmallInt)]
pub enum CgroupnsMode {
    /// Empty restart policy
    Empty = 0,
    /// No restart policy
    Private = 1,
    /// Always restart the container
    Host = 2,
}

impl Display for CgroupnsMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CgroupnsMode::Empty => write!(f, ""),
            CgroupnsMode::Private => write!(f, "private"),
            CgroupnsMode::Host => write!(f, "host"),
        }
    }
}

impl FromStr for CgroupnsMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let res = match s {
            "" => CgroupnsMode::Empty,
            "private" => CgroupnsMode::Private,
            "host" => CgroupnsMode::Host,
            _ => return Err(format!("couldn't parse cgroupns {s}")),
        };

        Ok(res)
    }
}

impl From<CgroupnsMode> for i16 {
    fn from(value: CgroupnsMode) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i16> for CgroupnsMode {
    type Error = String;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(CgroupnsMode::Empty),
            1 => Ok(CgroupnsMode::Private),
            2 => Ok(CgroupnsMode::Host),
            _ => Err(format!("unrecognized cgroupns mode value {value}")),
        }
    }
}

impl FromSql<SmallInt, Sqlite> for CgroupnsMode {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i16::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<SmallInt, Sqlite> for CgroupnsMode {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i16::from(*self);

        out.set_value(i32::from(val));

        Ok(IsNull::No)
    }
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_images)]
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerEnv<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Environment variable name and optionally a value
    pub value: Cow<'a, str>,
}

/// Container commands
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_cmds)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerCmd<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Idx in the db
    pub idx: i64,
    /// Command
    pub cmd: Cow<'a, str>,
}

/// Container test command for the health check
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_healthcheck_test)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerHealthcheckTest<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Idx in the db
    pub idx: i64,
    /// Command
    pub cmd: Cow<'a, str>,
}

/// Container entrypoint
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_entrypoints)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerEntrypoint<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Idx in the db
    pub idx: i64,
    /// Command
    pub cmd: Cow<'a, str>,
}

/// Container labels
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_labels)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerLabel<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Key of the label
    pub key: Cow<'a, str>,
    /// Label value
    pub value: Cow<'a, str>,
}

/// Container exposed ports
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_exposed_ports)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerExposedPort<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Port
    pub port: Cow<'a, str>,
}

/// Container cgroup rules
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_device_cgroup_rules)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDeviceCgroupRule<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Cgroup rule
    pub rule: Cow<'a, str>,
}

/// Container ulimit
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_ulimits)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerUlimit<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Name of the ulimit
    pub name: Cow<'a, str>,
    /// Soft limit
    pub soft: i32,
    /// Hard limit
    pub hard: i32,
}

/// Container dns
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_dns)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDns<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Dns server
    pub dns: Cow<'a, str>,
}

/// Container dns option
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_dns_options)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDnsOption<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Dns server
    pub dns_option: Cow<'a, str>,
}

/// Container dns search adress
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_dns_search)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDnsSearch<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Dns search address
    pub dns_search: Cow<'a, str>,
}

/// Container group add
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_group_add)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerGroupAdd<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Dns search address
    pub group_add: Cow<'a, str>,
}

/// Container sysctl
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_sysctls)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerSysctl<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// System parameter name
    pub key: Cow<'a, str>,
    /// System parameter value
    pub value: Cow<'a, str>,
}

/// Container log config
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_log_config)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerLogConfig<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// System parameter name
    pub key: Cow<'a, str>,
    /// System parameter value
    pub value: Cow<'a, str>,
}

/// Container blkio weight  device
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_blkio_weight_device)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBlkioWeightDevice<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Device path
    pub path: Cow<'a, str>,
    /// Weight
    pub weight: i32,
}

/// Container blkio device read bps
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_blkio_device_read_bps)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBlkioDeviceReadBps<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Device path
    pub path: Cow<'a, str>,
    /// Rate
    pub rate: i64,
}

/// Container blkio device write bps
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_blkio_device_write_bps)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBlkioDeviceWriteBps<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Device path
    pub path: Cow<'a, str>,
    /// Rate
    pub rate: i64,
}

/// Container blkio device read iops
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_blkio_device_read_iops)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBlkioDeviceReadIops<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Device path
    pub path: Cow<'a, str>,
    /// Rate
    pub rate: i64,
}

/// Container blkio device write iops
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_blkio_device_write_iops)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBlkioDeviceWriteIops<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Device path
    pub path: Cow<'a, str>,
    /// Rate
    pub rate: i64,
}

/// Container security opts
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq, PartialOrd, Ord)]
#[diesel(table_name = crate::schema::containers::container_securityopts)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerSecurityopt<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Security option
    pub opt: Cow<'a, str>,
}

/// Container masked path
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_masked_paths)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMaskedPath<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Path to mask
    pub path: Cow<'a, str>,
}

/// Container read only path
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_readonly_paths)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerReadonlyPath<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Path to mask
    pub path: Cow<'a, str>,
}

/// Bind mounts for a container
#[derive(Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_binds)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerBind<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Environment variable name and optionally a value
    pub value: Cow<'a, str>,
}

/// Container port bindings
#[derive(Debug, Clone, Insertable, HasQuery, Associations, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_port_bindings)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerPortBind<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Container port and optionally protocol
    pub port: Cow<'a, str>,
    /// Index o the array of host ip and port for the container port
    pub idx: i64,
    /// Host IP to map the port to
    pub host_ip: Option<Cow<'a, str>>,
    /// Host port to map the port to
    pub host_port: Option<HostPort>,
}

/// Container port bindings
#[derive(Debug, Clone, HasQuery, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_port_bindings)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerPortBindData<'a> {
    /// Container port and optionally protocol
    pub port: Cow<'a, str>,
    /// Host IP to map the port to
    pub host_ip: Option<Cow<'a, str>>,
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
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerExtraHost<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Host to add to /etc/hosts inside the container.
    ///
    /// Its in the form of Host:IP
    pub value: Cow<'a, str>,
}

/// A kernel capabilities to add to the container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_add_capabilities)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerAddCapability<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// A kernel capabilities to add to the container.
    pub value: Cow<'a, str>,
}

/// A kernel capabilities to drop to the container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_drop_capabilities)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDropCapability<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// A kernel capabilities to drop from the container.
    pub value: Cow<'a, str>,
}

/// Storage driver options for a container.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_storage_options)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerStorageOptions<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Name of the option
    ///
    /// Example: `size`
    pub name: Cow<'a, str>,
    /// The value of the option
    ///
    /// Example: `120G`
    pub value: Cow<'a, str>,
}

/// A map of container directories which should be replaced by tmpfs mounts.
#[derive(
    Debug, Clone, Insertable, Queryable, Associations, Selectable, PartialEq, Eq, PartialOrd, Ord,
)]
#[diesel(table_name = crate::schema::containers::container_tmpfs)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerTmpfs<'a> {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// Path inside the container.
    ///
    /// Example: `/run`
    pub path: Cow<'a, str>,
    /// Mount options for the tmpfs
    ///
    /// Example: `rw,noexec,nosuid,size=65536k`
    pub options: Cow<'a, str>,
}

/// Device Mapping used by a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_device_mappings)]
#[diesel(belongs_to(Container<'_>))]
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
#[diesel(belongs_to(Container<'_>))]
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

/// device request used by a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_device_requests)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(belongs_to(DeviceRequest))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerDeviceRequest {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`DeviceRequest`] id
    pub device_request_id: SqlUuid,
}

/// Missing device request for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::container_missing_device_requests)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingDeviceRequest {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`DeviceRequest`] id
    pub device_request_id: SqlUuid,
}

type ContainerMissingDeviceRequestByDeviceRequest<'a> =
    Eq<container_missing_device_requests::device_request_id, &'a SqlUuid>;
type ContainerMissingDeviceRequestFilterByDeviceRequest<'a> = Filter<
    container_missing_device_requests::table,
    ContainerMissingDeviceRequestByDeviceRequest<'a>,
>;

impl ContainerMissingDeviceRequest {
    /// Returns the filter container_missing_device_request table by id.
    pub fn by_device_request(
        device_request_id: &SqlUuid,
    ) -> ContainerMissingDeviceRequestByDeviceRequest<'_> {
        container_missing_device_requests::device_request_id.eq(device_request_id)
    }

    /// Returns the filtered container_missing_device_request table by id.
    pub fn find_by_device_request(
        device_request_id: &SqlUuid,
    ) -> ContainerMissingDeviceRequestFilterByDeviceRequest<'_> {
        container_missing_device_requests::table.filter(Self::by_device_request(device_request_id))
    }
}

impl From<ContainerDeviceRequest> for ContainerMissingDeviceRequest {
    fn from(
        ContainerDeviceRequest {
            container_id,
            device_request_id,
        }: ContainerDeviceRequest,
    ) -> Self {
        Self {
            container_id,
            device_request_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[rstest::rstest]
    #[case(ContainerStatus::Received)]
    #[case(ContainerStatus::Published)]
    #[case(ContainerStatus::Stopped)]
    #[case(ContainerStatus::Running)]
    fn should_convert_status(#[case] exp: ContainerStatus) {
        let val = i32::from(exp);

        let status = ContainerStatus::try_from(val).unwrap();

        assert_eq!(status, exp);
    }

    #[rstest::rstest]
    #[case(ContainerRestartPolicy::Empty)]
    #[case(ContainerRestartPolicy::No)]
    #[case(ContainerRestartPolicy::Always)]
    #[case(ContainerRestartPolicy::UnlessStopped)]
    #[case(ContainerRestartPolicy::OnFailure)]
    fn should_convert_restart_policy(#[case] exp: ContainerRestartPolicy) {
        let val = i32::from(exp);

        let status = ContainerRestartPolicy::try_from(val).unwrap();

        assert_eq!(status, exp);
    }

    #[rstest::rstest]
    #[case(CgroupnsMode::Empty)]
    #[case(CgroupnsMode::Private)]
    #[case(CgroupnsMode::Host)]
    fn should_convert_cgroupns_mode(#[case] exp: CgroupnsMode) {
        let val = i16::from(exp);

        let status = CgroupnsMode::try_from(val).unwrap();

        assert_eq!(status, exp);
    }
    #[rstest::rstest]
    #[case("", ContainerRestartPolicy::Empty)]
    #[case("no", ContainerRestartPolicy::No)]
    #[case("unless-stopped", ContainerRestartPolicy::UnlessStopped)]
    #[case("on-failure", ContainerRestartPolicy::OnFailure)]
    #[case("on-failure", ContainerRestartPolicy::OnFailure)]
    fn parse_restart_policy(#[case] case: &str, #[case] exp: ContainerRestartPolicy) {
        let policy = ContainerRestartPolicy::from_str(case).unwrap();

        assert_eq!(policy, exp);
        assert_eq!(policy.to_string(), case);
    }
}
