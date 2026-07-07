// This file is part of Edgehog.
//
// Copyright 2023-2026 SECO Mind Srl
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

//! Docker struct to manage containers.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::ops::{Deref, DerefMut};
use std::sync::OnceLock;

use bollard::errors::Error as BollardError;
use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, ContainerStatsResponse, EndpointSettings,
    HostConfig, PortBinding, RestartPolicy as BollardRestartPolicy, RestartPolicyNameEnum,
};
use bollard::plugin::{
    DeviceMapping, DeviceRequest, HealthConfig, HostConfigCgroupnsModeEnum, HostConfigLogConfig,
    ResourcesBlkioWeightDevice, ResourcesUlimits, ThrottleDevice,
};
use bollard::query_parameters::{
    CreateContainerOptions, InspectContainerOptions, RemoveContainerOptions, StartContainerOptions,
    StatsOptionsBuilder, StopContainerOptions,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::containers::container::{
    CgroupnsMode, Container as StoreContainer, ContainerBlkioDeviceReadBps,
    ContainerBlkioDeviceReadIops, ContainerBlkioDeviceWriteBps, ContainerBlkioDeviceWriteIops,
    ContainerBlkioWeightDevice, ContainerPortBindData, ContainerRestartPolicy, ContainerUlimit,
};
use edgehog_store::models::containers::device_mapping::DeviceMapping as StoreDeviceMapping;
use eyre::Context;
use futures::StreamExt;
use tracing::{debug, info, instrument, trace, warn};
use uuid::Uuid;

use crate::client::*;
use crate::store::device_request::StoredDeviceRequest;

/// Error for the container operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ContainerError {
    /// couldn't create the container
    Create(#[source] BollardError),
    /// couldn't inspect the container
    Inspect(#[source] BollardError),
    /// couldn't remove the container
    Remove(#[source] BollardError),
    /// couldn't start container.
    Start(#[source] BollardError),
    /// couldn't stop container
    Stop(#[source] BollardError),
    /// missing image reference in container definition
    Image,
}

/// Identifies a container univocally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerId {
    /// Id of the docker container.
    ///
    /// The id of the image is optional since it will be available only when the image is created.
    pub(crate) id: Option<String>,
    /// Assign the specified name to the container.
    ///
    /// Must match /?[a-zA-Z0-9][a-zA-Z0-9_.-]+.
    pub(crate) name: Uuid,
    /// Cache the name with a single allocation.
    ///
    /// Usually multiple functions are called in sequence.
    name_cache: OnceLock<String>,
}

impl ContainerId {
    pub(crate) fn new(id: Option<String>, name: Uuid) -> Self {
        Self {
            id,
            name,
            name_cache: OnceLock::new(),
        }
    }

    pub(crate) fn name_as_str(&self) -> &str {
        self.name_cache
            .get_or_init(|| self.name.to_string())
            .as_str()
    }

    /// Get the container id or name if it's missing.
    #[instrument(skip_all)]
    pub(crate) fn container(&self) -> &str {
        match &self.id {
            Some(id) => {
                trace!("returning id");

                id.as_str()
            }
            None => {
                trace!("id missing, returning name");

                self.name_as_str()
            }
        }
    }

    /// Set the id from docker.
    #[instrument(skip_all)]
    fn update(&mut self, id: String) {
        info!("using id {id} for container {}", self.name);

        let old_id = self.id.replace(id);

        trace!(?old_id);
    }

    pub(crate) async fn inspect(
        &mut self,
        client: &Client,
    ) -> Result<Option<ContainerInspectResponse>, ContainerError> {
        // We need to account to the case that we have an incorrect id, but it exists another
        // container with the correct name
        if let Some(id) = self.id.clone() {
            debug!("checking the id");

            let response = self.inspect_with(client, &id).await?;

            if response.is_some() {
                return Ok(response);
            }
        }
        // Use a variable to circumvent a bug in clippy
        let name = self.name_as_str().to_string();
        self.inspect_with(client, &name).await
    }

    /// Inspect a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerInspect)
    #[instrument(skip_all)]
    async fn inspect_with(
        &mut self,
        client: &Client,
        name: &str,
    ) -> Result<Option<ContainerInspectResponse>, ContainerError> {
        debug!("Inspecting the {}", self);

        let res = client
            .inspect_container(name, None::<InspectContainerOptions>)
            .await;

        let container = match res {
            Ok(container) => container,
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(ContainerError::Inspect(err)),
        };

        trace!("container info: {container:?}");

        if let Some(id) = &container.id {
            self.update(id.clone());
        }

        Ok(Some(container))
    }

    /// Remove a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerDelete)
    #[instrument(skip_all)]
    pub(crate) async fn remove(&self, client: &Client) -> Result<Option<()>, ContainerError> {
        debug!("deleting {}", self);

        let opts = RemoveContainerOptions {
            v: false,
            // TODO: there is no way to force the remove from astarte
            force: false,
            link: false,
        };

        let res = client.remove_container(self.container(), Some(opts)).await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Remove(err)),
        }
    }

    /// Start a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerStart)
    #[instrument(skip_all)]
    pub(crate) async fn start(&self, client: &Client) -> Result<Option<()>, ContainerError> {
        debug!("starting {self}");

        let res = client
            .start_container(self.container(), None::<StartContainerOptions>)
            .await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Start(err)),
        }
    }

    /// Stop a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerStop)
    #[instrument(skip_all)]
    pub(crate) async fn stop(&self, client: &Client) -> Result<Option<()>, ContainerError> {
        debug!("stopping {self}");

        let res = client
            .stop_container(self.container(), None::<StopContainerOptions>)
            .await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 304,
                message,
            }) => {
                debug!("container already stopped: {message}");

                Ok(Some(()))
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Start(err)),
        }
    }

    /// Stats of docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerStats)
    #[instrument(skip_all)]
    pub(crate) async fn stats(
        &self,
        client: &Client,
    ) -> Result<Option<ContainerStatsResponse>, ContainerError> {
        debug!("getting statistics {self}");
        let options = StatsOptionsBuilder::new()
            .stream(false)
            // This sets where we get a single stat instead of waiting for 2 cycles
            // We want 2 cycles for pre cpu usages
            .one_shot(false)
            .build();
        let res = client.stats(self.container(), Some(options)).next().await;

        let Some(res) = res else {
            return Ok(None);
        };

        match res {
            Ok(stats) => Ok(Some(stats)),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Start(err)),
        }
    }
}

impl Display for ContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(id) = &self.id {
            write!(f, "id: {id}, ")?;
        }

        write!(f, "name: {}", self.name)
    }
}

/// Docker container struct.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Container {
    pub(crate) id: ContainerId,
    pub(crate) req: Box<ContainerCreateBody>,
}

impl Container {
    pub(crate) fn add_image(&mut self, refrence: String) {
        self.req.image = Some(refrence);
    }

    pub(crate) fn add_networks(&mut self, network_ids: Vec<SqlUuid>) {
        let endpoint_config = self
            .req
            .networking_config
            .get_or_insert_default()
            .endpoints_config
            .get_or_insert_default();

        let iter = network_ids.into_iter().map(|net_id| {
            (
                net_id.to_string(),
                EndpointSettings {
                    ..Default::default()
                },
            )
        });

        endpoint_config.extend(iter);
    }

    pub(crate) fn add_env_vars(&mut self, envs: Vec<String>) {
        self.req.env.get_or_insert_default().extend(envs);
    }

    pub(crate) fn add_binds(&mut self, binds: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .binds
            .get_or_insert_default()
            .extend(binds);
    }

    /// Convert the port bindings to be used in [`HostConfig`].
    pub(crate) fn add_port_bindings(&mut self, binds: Vec<ContainerPortBindData<'_>>) {
        let exposed_ports = binds.iter().map(|bind| bind.port.to_string());

        self.add_exposed_ports(exposed_ports);

        let port_binds = self
            .req
            .host_config
            .get_or_insert_default()
            .port_bindings
            .get_or_insert_default();

        binds.into_iter().for_each(|bind| {
            let ContainerPortBindData {
                port,
                host_ip,
                host_port,
            } = bind;

            let entry = port_binds
                .entry(port.into_owned())
                .or_default()
                .get_or_insert_default();

            entry.push(PortBinding {
                host_ip: host_ip.map(Cow::into_owned),
                host_port: host_port.map(|port| port.to_string()),
            });
        });
    }

    pub(crate) fn add_extra_hosts(&mut self, hosts: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .extra_hosts
            .get_or_insert_default()
            .extend(hosts);
    }

    pub(crate) fn extend_caps(&mut self, cap_add: Vec<String>, cap_drop: Vec<String>) {
        let host = self.req.host_config.get_or_insert_default();

        host.cap_add.get_or_insert_default().extend(cap_add);
        host.cap_drop.get_or_insert_default().extend(cap_drop);
    }

    pub(crate) fn add_storage_opt(&mut self, opts: HashMap<String, String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .storage_opt
            .get_or_insert_default()
            .extend(opts);
    }

    pub(crate) fn add_tempfs(&mut self, tmpfs: HashMap<String, String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .tmpfs
            .get_or_insert_default()
            .extend(tmpfs);
    }

    pub(crate) fn add_device_mappings(&mut self, device_mappings: Vec<StoreDeviceMapping>) {
        let iter = device_mappings.into_iter().map(|mapping| DeviceMapping {
            path_on_host: Some(mapping.path_on_host),
            path_in_container: Some(mapping.path_in_container),
            cgroup_permissions: mapping.cgroup_permissions,
        });

        self.req
            .host_config
            .get_or_insert_default()
            .devices
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_device_request(&mut self, req: StoredDeviceRequest) {
        self.req
            .host_config
            .get_or_insert_default()
            .device_requests
            .get_or_insert_default()
            .push(DeviceRequest {
                driver: req.device_request.driver,
                count: req.device_request.count,
                device_ids: Some(req.device_ids),
                capabilities: Some(req.capabilities),
                options: Some(req.options),
            });
    }

    pub(crate) fn add_cmd(&mut self, cmds: Vec<String>) {
        self.req.cmd.get_or_insert_default().extend(cmds);
    }

    pub(crate) fn add_health_check_test(&mut self, test: Vec<String>) {
        self.req
            .healthcheck
            .get_or_insert_default()
            .test
            .get_or_insert_default()
            .extend(test);
    }

    pub(crate) fn add_entrypoint(&mut self, entrypoint: Vec<String>) {
        self.req
            .entrypoint
            .get_or_insert_default()
            .extend(entrypoint);
    }

    pub(crate) fn add_labels(&mut self, labels: HashMap<String, String>) {
        self.req.labels.get_or_insert_default().extend(labels);
    }

    pub(crate) fn add_exposed_ports(&mut self, ports: impl IntoIterator<Item = String>) {
        self.req.exposed_ports.get_or_insert_default().extend(ports);
    }

    pub(crate) fn add_device_cgroup_rules(&mut self, rules: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .device_cgroup_rules
            .get_or_insert_default()
            .extend(rules);
    }

    pub(crate) fn add_ulimits(&mut self, ulimits: Vec<ContainerUlimit>) {
        let iter = ulimits.into_iter().map(
            |ContainerUlimit {
                 container_id: _,
                 name,
                 soft,
                 hard,
             }| ResourcesUlimits {
                name: Some(name.into_owned()),
                soft: Some(i64::from(soft)),
                hard: Some(i64::from(hard)),
            },
        );

        self.req
            .host_config
            .get_or_insert_default()
            .ulimits
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_dns(&mut self, dns: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .dns
            .get_or_insert_default()
            .extend(dns);
    }

    pub(crate) fn add_dns_options(&mut self, options: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .dns_options
            .get_or_insert_default()
            .extend(options);
    }

    pub(crate) fn add_dns_search(&mut self, search: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .dns_search
            .get_or_insert_default()
            .extend(search);
    }

    pub(crate) fn add_group_add(&mut self, groups: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .group_add
            .get_or_insert_default()
            .extend(groups);
    }

    pub(crate) fn add_sysctl(&mut self, systctl: HashMap<String, String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .sysctls
            .get_or_insert_default()
            .extend(systctl);
    }

    pub(crate) fn add_log_config(&mut self, log_config: HashMap<String, String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .log_config
            .get_or_insert_default()
            .config
            .get_or_insert_default()
            .extend(log_config);
    }

    pub(crate) fn add_blkio_weight_device(
        &mut self,
        blkio: Vec<ContainerBlkioWeightDevice>,
    ) -> eyre::Result<()> {
        let iter = blkio
            .into_iter()
            .map(|item| {
                let weight = usize::try_from(item.weight)
                    .wrap_err("couldn't convert device blkio weight")?;

                Ok(ResourcesBlkioWeightDevice {
                    path: Some(item.path.into_owned()),
                    weight: Some(weight),
                })
            })
            .collect::<eyre::Result<Vec<ResourcesBlkioWeightDevice>>>()?;

        self.req
            .host_config
            .get_or_insert_default()
            .blkio_weight_device
            .get_or_insert_default()
            .extend(iter);

        Ok(())
    }

    pub(crate) fn add_blkio_device_read_bps(&mut self, blkio: Vec<ContainerBlkioDeviceReadBps>) {
        let iter = blkio.into_iter().map(|item| ThrottleDevice {
            path: Some(item.path.into_owned()),
            rate: Some(item.rate),
        });

        self.req
            .host_config
            .get_or_insert_default()
            .blkio_device_read_bps
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_blkio_device_write_bps(&mut self, blkio: Vec<ContainerBlkioDeviceWriteBps>) {
        let iter = blkio.into_iter().map(|item| ThrottleDevice {
            path: Some(item.path.into_owned()),
            rate: Some(item.rate),
        });

        self.req
            .host_config
            .get_or_insert_default()
            .blkio_device_write_bps
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_blkio_device_read_iops(&mut self, blkio: Vec<ContainerBlkioDeviceReadIops>) {
        let iter = blkio.into_iter().map(|item| ThrottleDevice {
            path: Some(item.path.into_owned()),
            rate: Some(item.rate),
        });

        self.req
            .host_config
            .get_or_insert_default()
            .blkio_device_read_iops
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_blkio_device_write_iops(
        &mut self,
        blkio: Vec<ContainerBlkioDeviceWriteIops>,
    ) {
        let iter = blkio.into_iter().map(|item| ThrottleDevice {
            path: Some(item.path.into_owned()),
            rate: Some(item.rate),
        });

        self.req
            .host_config
            .get_or_insert_default()
            .blkio_device_write_iops
            .get_or_insert_default()
            .extend(iter);
    }

    pub(crate) fn add_security_opts(&mut self, security_opts: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .security_opt
            .get_or_insert_default()
            .extend(security_opts);
    }

    pub(crate) fn add_masked_paths(&mut self, masked_paths: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .masked_paths
            .get_or_insert_default()
            .extend(masked_paths);
    }

    pub(crate) fn add_read_only_paths(&mut self, read_only_paths: Vec<String>) {
        self.req
            .host_config
            .get_or_insert_default()
            .readonly_paths
            .get_or_insert_default()
            .extend(read_only_paths);
    }

    /// Create a new docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerCreate)
    #[instrument(skip_all)]
    pub async fn create(&mut self, client: &Client) -> Result<(), ContainerError> {
        debug!("creating the {}", self);

        let options = CreateContainerOptions::from(&*self);
        let config = self.req.clone();
        let res = client
            .create_container(Some(options), *config)
            .await
            .map_err(ContainerError::Create)?;

        self.id.update(res.id);

        for warning in res.warnings {
            warn!("container created with working: {warning}");
        }

        Ok(())
    }
}

impl Display for Container {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // brackets go brrrrrr
        write!(f, "Container {{{}}}", self.id)
    }
}

impl Deref for Container {
    type Target = ContainerId;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl DerefMut for Container {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.id
    }
}

impl<'a> From<&'a Container> for CreateContainerOptions {
    fn from(value: &'a Container) -> Self {
        CreateContainerOptions {
            name: Some(value.name_as_str().to_string()),
            platform: String::new(),
        }
    }
}

impl<'a> TryFrom<StoreContainer<'a>> for Container {
    type Error = eyre::Report;

    fn try_from(value: StoreContainer<'a>) -> Result<Self, Self::Error> {
        let StoreContainer {
            id,
            local_id,
            image_id: _,
            status: _,
            network_mode,
            hostname,
            restart_policy,
            restart_policy_maximum_retry_count,
            cpu_period,
            cpu_quota,
            cpu_realtime_period,
            cpu_realtime_runtime,
            cpu_shares,
            memory,
            memory_reservation,
            memory_swap,
            memory_swappiness,
            volume_driver,
            read_only_rootfs,
            privileged,
            domainname,
            user,
            health_check_interval,
            health_check_timeout,
            health_check_retries,
            health_check_start_period,
            health_check_start_interval,
            working_dir,
            network_disabled,
            stop_signal,
            stop_timeout,
            auto_remove,
            cgroupns_mode,
            ipc_mode,
            oom_score_adj,
            userns_mode,
            shm_size,
            log_type,
            blkio_weight,
            pid_mode,
            runtime,
        } = value;

        let restart_policy = restart_policy.map(|policy| BollardRestartPolicy {
            name: Some(match policy {
                ContainerRestartPolicy::Empty => RestartPolicyNameEnum::EMPTY,
                ContainerRestartPolicy::No => RestartPolicyNameEnum::NO,
                ContainerRestartPolicy::Always => RestartPolicyNameEnum::ALWAYS,
                ContainerRestartPolicy::UnlessStopped => RestartPolicyNameEnum::UNLESS_STOPPED,
                ContainerRestartPolicy::OnFailure => RestartPolicyNameEnum::ON_FAILURE,
            }),
            maximum_retry_count: restart_policy_maximum_retry_count.map(i64::from),
        });

        let cgroupns_mode = cgroupns_mode.map(|mode| match mode {
            CgroupnsMode::Empty => HostConfigCgroupnsModeEnum::EMPTY,
            CgroupnsMode::Private => HostConfigCgroupnsModeEnum::PRIVATE,
            CgroupnsMode::Host => HostConfigCgroupnsModeEnum::HOST,
        });

        let log_config = log_type.map(|log_type| HostConfigLogConfig {
            typ: Some(log_type.into_owned()),
            config: None,
        });

        let blkio_weight = blkio_weight
            .map(u16::try_from)
            .transpose()
            .wrap_err("invalid blkio_weight value")?;

        Ok(Container {
            id: ContainerId::new(local_id.map(Cow::into_owned), id.0),
            req: Box::new(ContainerCreateBody {
                hostname: hostname.map(Cow::into_owned),
                domainname: domainname.map(Cow::into_owned),
                user: user.map(Cow::into_owned),
                working_dir: working_dir.map(Cow::into_owned),
                network_disabled,
                stop_signal: stop_signal.map(|sig| sig.to_string()),
                stop_timeout: stop_timeout.map(i64::from),
                healthcheck: Some(HealthConfig {
                    test: None,
                    interval: health_check_interval,
                    timeout: health_check_timeout,
                    retries: health_check_retries.map(i64::from),
                    start_period: health_check_start_period,
                    start_interval: health_check_start_interval,
                }),
                host_config: Some(HostConfig {
                    network_mode: network_mode.map(Cow::into_owned),
                    restart_policy,
                    cpu_period,
                    cpu_quota,
                    cpu_realtime_period,
                    cpu_realtime_runtime,
                    cpu_shares: cpu_shares.map(i64::from),
                    memory,
                    memory_reservation,
                    memory_swap,
                    memory_swappiness: memory_swappiness.map(i64::from),
                    volume_driver: volume_driver.map(Cow::into_owned),
                    readonly_rootfs: read_only_rootfs,
                    privileged,
                    auto_remove,
                    cgroupns_mode,
                    ipc_mode: ipc_mode.map(Cow::into_owned),
                    oom_score_adj: oom_score_adj.map(i64::from),
                    userns_mode: userns_mode.map(Cow::into_owned),
                    shm_size,
                    log_config,
                    blkio_weight,
                    pid_mode: pid_mode.map(Cow::into_owned),
                    runtime: runtime.map(Cow::into_owned),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        })
    }
}

fn opt_eq<T, U>(opt1: &Option<T>, opt2: &Option<U>) -> bool
where
    T: PartialEq<U>,
{
    match (opt1, opt2) {
        (None, None) => true,
        (None, Some(_)) | (Some(_), None) => false,
        (Some(v1), Some(v2)) => *v1 == *v2,
    }
}

/// Represents a binding between a host IP address and a host port.
#[derive(Debug, Clone, Eq)]
pub struct Binding<S = String> {
    /// Host IP
    pub host_ip: Option<S>,
    /// Host port
    pub host_port: Option<u16>,
}

impl<S> From<&Binding<S>> for PortBinding
where
    S: AsRef<str>,
{
    fn from(value: &Binding<S>) -> Self {
        let host_ip = value.host_ip.as_ref().map(|s| s.as_ref().to_string());
        let host_port = value.host_port.map(|p| p.to_string());

        PortBinding { host_ip, host_port }
    }
}

impl<'a> From<&'a Binding<String>> for Binding<&'a str> {
    fn from(value: &'a Binding<String>) -> Self {
        Binding {
            host_ip: value.host_ip.as_deref(),
            host_port: value.host_port,
        }
    }
}

impl From<Binding<&str>> for Binding<String> {
    fn from(value: Binding<&str>) -> Self {
        Binding {
            host_ip: value.host_ip.map(ToString::to_string),
            host_port: value.host_port,
        }
    }
}

impl<S> Display for Binding<S>
where
    S: AsRef<str>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.host_ip, self.host_port) {
            (None, None) => Ok(()),
            (Some(ip), None) => write!(f, "{}", ip.as_ref()),
            (None, Some(port)) => write!(f, "{port}"),
            (Some(ip), Some(port)) => write!(f, "{}:{port}", ip.as_ref()),
        }
    }
}

impl<S1, S2> PartialEq<Binding<S2>> for Binding<S1>
where
    S1: PartialEq<S2>,
{
    fn eq(&self, Binding { host_ip, host_port }: &Binding<S2>) -> bool {
        opt_eq(&self.host_ip, host_ip) && self.host_port.eq(host_port)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use bollard::plugin::NetworkingConfig;
    use mockall::predicate;

    use crate::requests::container::CreateContainer;
    use crate::{docker_mock, image::Image};

    use super::*;

    pub(crate) fn create_container_resource(container: &CreateContainer) -> Container {
        Container {
            id: ContainerId::new(None, container.id.0),
            req: Box::new(ContainerCreateBody {
                image: Some("postgres:15".to_string()),
                // network_ids: Some(VecReqUuid(vec![network.id])),
                // volume_ids: Some(VecReqUuid(vec![volume.id])),
                // device_mapping_ids: Some(VecReqUuid(vec![device_mapping.id])),
                // device_request_ids: Some(VecReqUuid(vec![device_request.id])),
                domainname: Some("database.host".to_string()),
                hostname: Some("database".to_string()),
                env: Some(
                    ["POSTGRES_PASSWORD=password", "POSTGRES_USER=user"]
                        .map(str::to_string)
                        .to_vec(),
                ),
                user: Some("pg".to_string()),
                cmd: Some(["-u=postgres", "-d=test"].map(str::to_string).to_vec()),
                working_dir: Some("/app-data".to_string()),
                entrypoint: Some(["CMD-SHELL", "/entrypoint.sh"].map(str::to_string).to_vec()),
                network_disabled: Some(false),
                healthcheck: Some(HealthConfig {
                    test: Some(
                        ["pg-is-ready", "-u=postgres", "-d=test"]
                            .map(str::to_string)
                            .to_vec(),
                    ),
                    interval: Some(42_000),
                    timeout: Some(6_000),
                    retries: Some(3),
                    start_period: Some(10_000),
                    start_interval: Some(10_000),
                }),
                labels: Some(HashMap::from_iter(
                    [("lab_1", "foo"), ("lab_2", "bar")]
                        .map(|(k, v)| (k.to_string(), v.to_string())),
                )),
                stop_signal: Some("SIGSTOP".to_string()),
                stop_timeout: Some(32),
                exposed_ports: Some(
                    ["5432", "53/udp", "90/tcp", "9000"]
                        .map(str::to_string)
                        .to_vec(),
                ),
                host_config: Some(HostConfig {
                    restart_policy: Some(BollardRestartPolicy {
                        name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                        maximum_retry_count: Some(4),
                    }),
                    binds: Some(vec!["/var/lib/postgres:/data".to_string()]),
                    network_mode: Some("bridge".to_string()),
                    port_bindings: Some(HashMap::from_iter([(
                        "5432".to_string(),
                        Some(vec![PortBinding {
                            host_port: Some("5432".to_string()),
                            host_ip: None,
                        }]),
                    )])),
                    extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
                    cap_add: Some(vec!["CAP_CHOWN".to_string()]),
                    cap_drop: Some(vec!["CAP_KILL".to_string()]),
                    cpu_period: Some(1000),
                    cpu_quota: Some(100),
                    cpu_realtime_period: Some(1000),
                    cpu_realtime_runtime: Some(100),
                    memory: Some(4096),
                    memory_reservation: Some(1024),
                    memory_swap: Some(8192),
                    memory_swappiness: Some(50),
                    volume_driver: Some("local".to_string()),
                    storage_opt: Some(HashMap::from_iter([(
                        "size".to_string(),
                        "1024k".to_string(),
                    )])),
                    readonly_rootfs: Some(true),
                    tmpfs: Some(HashMap::from_iter([(
                        "/run".to_string(),
                        "rw,noexec,nosuid,size=65536k".to_string(),
                    )])),
                    privileged: Some(false),
                    cpu_shares: Some(100),
                    device_cgroup_rules: Some(vec!["c 1:3 mr".to_string()]),
                    ulimits: Some(vec![ResourcesUlimits {
                        name: Some("nofile".to_string()),
                        soft: Some(1024),
                        hard: Some(2048),
                    }]),
                    auto_remove: Some(false),
                    cgroupns_mode: Some(HostConfigCgroupnsModeEnum::PRIVATE),
                    dns: Some(vec!["8.8.8.8".to_string()]),
                    dns_options: Some(vec!["timeout:3".to_string()]),
                    dns_search: Some(vec!["example.com".to_string()]),
                    group_add: Some(vec!["dialout".to_string()]),
                    ipc_mode: Some("shareable".to_string()),
                    oom_score_adj: Some(500),
                    userns_mode: Some("host".to_string()),
                    sysctls: Some(HashMap::from_iter([(
                        "net.ipv4.ip_forward".to_string(),
                        "1".to_string(),
                    )])),
                    shm_size: Some(67_108_864), // 64 MB
                    runtime: Some("runc".to_string()),
                    log_config: Some(HostConfigLogConfig {
                        typ: Some("json-file".to_string()),
                        config: Some(HashMap::from_iter([
                            ("max-size".to_string(), "10m".to_string()),
                            ("max-file".to_string(), "3".to_string()),
                        ])),
                    }),
                    blkio_weight: Some(500),
                    blkio_weight_device: Some(vec![ResourcesBlkioWeightDevice {
                        path: Some("/dev/sda".to_string()),
                        weight: Some(500),
                    }]),
                    blkio_device_read_bps: Some(vec![ThrottleDevice {
                        path: Some("/dev/sda".to_string()),
                        rate: Some(1_048_576),
                    }]),
                    blkio_device_write_bps: Some(vec![ThrottleDevice {
                        path: Some("/dev/sda".to_string()),
                        rate: Some(1_048_576),
                    }]),
                    blkio_device_read_iops: Some(vec![ThrottleDevice {
                        path: Some("/dev/sda".to_string()),
                        rate: Some(1000),
                    }]),
                    blkio_device_write_iops: Some(vec![ThrottleDevice {
                        path: Some("/dev/sda".to_string()),
                        rate: Some(1000),
                    }]),
                    security_opt: Some(vec!["no-new-privileges".to_string()]),
                    pid_mode: Some("host".to_string()),
                    masked_paths: Some(vec!["/proc/kcore".to_string()]),
                    readonly_paths: Some(vec!["/proc/sys".to_string()]),
                    devices: Some(vec![bollard::models::DeviceMapping {
                        path_on_host: Some("/dev/tty12".to_string()),
                        path_in_container: Some("dev/tty12".to_string()),
                        cgroup_permissions: Some("msv".to_string()),
                    }]),
                    device_requests: Some(vec![bollard::models::DeviceRequest {
                        driver: Some("nvidia".to_string()),
                        count: Some(4),
                        device_ids: Some(
                            ["0", "1", "GPU-fef8089b-4820-abfc-e83e-94318197576e"]
                                .map(str::to_string)
                                .to_vec(),
                        ),
                        capabilities: Some(vec![
                            ["compute", "gpu", "nvidia"].map(str::to_string).to_vec(),
                        ]),
                        options: Some(HashMap::from_iter(
                            [("property1", "string"), ("property2", "string")]
                                .map(|(k, v)| (k.to_string(), v.to_string())),
                        )),
                    }]),
                    ..Default::default()
                }),
                networking_config: Some(NetworkingConfig {
                    endpoints_config: Some(HashMap::from_iter(
                        container
                            .network_ids
                            .as_ref()
                            .unwrap()
                            .iter()
                            .map(|id| (id.to_string(), EndpointSettings::default())),
                    )),
                }),
                ..Default::default()
            }),
        }
    }

    impl Container {
        fn new(name: Uuid, image: impl Into<String>) -> Self {
            Self {
                id: ContainerId::new(None, name),
                req: Box::new(ContainerCreateBody {
                    image: Some(image.into()),
                    ..Default::default()
                }),
            }
        }
    }

    #[tokio::test]
    async fn should_create() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == Some("hello-world:latest".to_string()))
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::models::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_str = name.to_string();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.name == Some(name_str.clone()))
                        && config.image == Some("hello-world:latest".to_string())
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            mock
        });

        let mut image = Image::new(None, "hello-world:latest", None);
        image.pull(&docker).await.unwrap();

        let mut container = Container::new(name, image.reference.clone());

        container.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == Some("hello-world:latest".to_string()))
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::models::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_cl = name.to_string();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.name == Some(name_cl.to_string()))
                        && config.image == Some("hello-world:latest".to_string())
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            let inspect_res = bollard::models::ContainerInspectResponse {
                id: Some("id".to_string()),
                name: Some(format!("/{name}")),
                image: Some("hello-world".to_string()),
                ..Default::default()
            };

            mock.expect_inspect_container()
                .withf(move |id, _option| id == "id")
                .once()
                .returning(move |_, _| Ok(inspect_res.clone()));

            mock
        });

        let mut image = Image::new(None, "hello-world:latest", None);
        image.pull(&docker).await.unwrap();

        let mut container = Container::new(name, image.reference.clone());

        container.create(&docker).await.unwrap();

        let resp = container.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(resp.name, Some(format!("/{name}")));
    }

    #[tokio::test]
    async fn should_inspect_not_found() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_inspect_container()
                .with(predicate::eq(name.to_string()), predicate::eq(None))
                .once()
                .returning(move |_, _| Err(crate::tests::not_found_response()));

            mock
        });

        let mut container = Container::new(name, "hello-world");

        let resp = container.inspect(&docker).await.unwrap();

        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn should_remove() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == Some("hello-world:latest".to_string()))
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::models::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_exp = name.to_string();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option
                        .as_ref()
                        .and_then(|opt| opt.name.as_ref())
                        .is_some_and(|name| *name == name_exp)
                        && config.image == Some("hello-world:latest".to_string())
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            mock.expect_remove_container()
                .withf(move |id, _options| id == "id")
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(()));

            mock
        });

        let mut image = Image::new(None, "hello-world:latest", None);
        image.pull(&docker).await.unwrap();

        let mut container = Container::new(name, image.reference.clone());

        container.create(&docker).await.unwrap();

        container.remove(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_remove_not_found() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_remove_container()
                .with(
                    predicate::eq(name.to_string()),
                    predicate::eq(Some(RemoveContainerOptions {
                        v: false,
                        force: false,
                        link: false,
                    })),
                )
                .once()
                .returning(move |_, _| Err(crate::tests::not_found_response()));

            mock
        });

        let container = Container::new(name, "hello-world");

        container.remove(&docker).await.unwrap();
    }

    #[test]
    fn to_string_bind() {
        let cases = [
            (
                Binding {
                    host_ip: Some("127.0.0.1"),
                    host_port: Some(80),
                },
                "127.0.0.1:80",
            ),
            (
                Binding {
                    host_ip: Some("127.0.0.1"),
                    host_port: None,
                },
                "127.0.0.1",
            ),
            (
                Binding {
                    host_ip: None,
                    host_port: Some(80),
                },
                "80",
            ),
        ];

        for (case, expect) in cases {
            assert_eq!(case.to_string(), expect)
        }
    }
}
