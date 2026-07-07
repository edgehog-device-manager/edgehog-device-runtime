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

use std::borrow::Cow;
use std::str::FromStr;

use diesel::{RunQueryDsl, delete, insert_or_ignore_into};

use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::container::{
    self, CgroupnsMode, Container, ContainerAddCapability, ContainerBind,
    ContainerBlkioDeviceReadBps, ContainerBlkioDeviceReadIops, ContainerBlkioDeviceWriteBps,
    ContainerBlkioDeviceWriteIops, ContainerBlkioWeightDevice, ContainerCmd,
    ContainerDeviceCgroupRule, ContainerDeviceMapping, ContainerDeviceRequest, ContainerDns,
    ContainerDnsOption, ContainerDnsSearch, ContainerDropCapability, ContainerEntrypoint,
    ContainerEnv, ContainerExposedPort, ContainerExtraHost, ContainerGroupAdd,
    ContainerHealthcheckTest, ContainerLabel, ContainerLogConfig, ContainerMaskedPath,
    ContainerMissingDeviceMapping, ContainerMissingDeviceRequest, ContainerMissingImage,
    ContainerMissingNetwork, ContainerMissingVolume, ContainerNetwork, ContainerPortBind,
    ContainerReadonlyPath, ContainerRestartPolicy, ContainerSecurityopt, ContainerStatus,
    ContainerStorageOptions, ContainerSysctl, ContainerTmpfs, ContainerUlimit, ContainerVolume,
    HostPort,
};
use edgehog_store::models::containers::deployment::DeploymentMissingContainer;
use edgehog_store::models::containers::device_mapping::DeviceMapping;
use edgehog_store::models::containers::device_request::DeviceRequest;
use edgehog_store::models::containers::image::Image;
use edgehog_store::models::containers::network::Network;
use edgehog_store::models::containers::volume::Volume;
use edgehog_store::schema::containers::{
    container_add_capabilities, container_binds, container_blkio_device_read_bps,
    container_blkio_device_read_iops, container_blkio_device_write_bps,
    container_blkio_device_write_iops, container_blkio_weight_device, container_cmds,
    container_device_cgroup_rules, container_device_mappings, container_device_requests,
    container_dns, container_dns_options, container_dns_search, container_drop_capabilities,
    container_entrypoints, container_env, container_exposed_ports, container_extra_hosts,
    container_group_add, container_healthcheck_test, container_labels, container_log_config,
    container_masked_paths, container_missing_device_mappings, container_missing_device_requests,
    container_missing_images, container_missing_networks, container_missing_volumes,
    container_networks, container_port_bindings, container_readonly_paths, container_securityopts,
    container_storage_options, container_sysctls, container_tmpfs, container_ulimits,
    container_volumes, containers, deployment_containers,
};
use tracing::{debug, instrument, warn};

use crate::requests::VecReqUuid;
use crate::requests::container::{CreateContainer, parse_port_binding};
use crate::store::{StateStore, StoreError};

impl StateStore {
    /// Stores the container received from the CreateRequest
    // XXX: separate in subfunction, this is tooo long
    #[instrument(skip_all, fields(id = %create_container.id))]
    pub(crate) async fn create_container(
        &self,
        create_container: Box<CreateContainer>,
    ) -> Result<(), StoreError> {
        self.handle
            .for_write(move |writer| {
                // this will make sure all the fields are used
                let CreateContainer {
                    id,
                    deployment_id: _,
                    hostname: _,
                    restart_policy: _,
                    network_mode: _,
                    cpu_period: _,
                    cpu_quota: _,
                    cpu_realtime_period: _,
                    cpu_realtime_runtime: _,
                    memory: _,
                    memory_reservation: _,
                    memory_swap: _,
                    memory_swappiness: _,
                    volume_driver: _,
                    read_only_rootfs: _,
                    privileged: _,
                    domainname: _,
                    user: _,
                    health_check_interval: _,
                    health_check_timeout: _,
                    health_check_retries: _,
                    health_check_start_period: _,
                    health_check_start_interval: _,
                    working_dir: _,
                    network_disabled: _,
                    stop_signal: _,
                    stop_timeout: _,
                    restart_policy_maximum_retry_count: _,
                    cpu_shares: _,
                    auto_remove: _,
                    cgroupns_mode: _,
                    ipc_mode: _,
                    oom_score_adj: _,
                    userns_mode: _,
                    shm_size: _,
                    runtime: _,
                    log_type: _,
                    blkio_weight: _,
                    pid_mode: _,
                    image_id,
                    network_ids,
                    volume_ids,
                    device_mapping_ids,
                    device_request_ids,
                    env,
                    binds,
                    port_bindings,
                    extra_hosts,
                    cap_add,
                    cap_drop,
                    storage_opt_keys,
                    storage_opt_values,
                    tmpfs_paths,
                    tmpfs_options,
                    cmd,
                    health_check_test,
                    entrypoint,
                    labels_keys,
                    labels_values,
                    exposed_ports,
                    device_cgroup_rules,
                    ulimits_name,
                    ulimits_soft,
                    ulimits_hard,
                    dns,
                    dns_options,
                    dns_search,
                    group_add,
                    sysctls_keys,
                    sysctls_values,
                    log_config_keys,
                    log_config_values,
                    blkio_weight_device_path,
                    blkio_weight_device_weight,
                    blkio_device_read_bps_path,
                    blkio_device_read_bps_rate,
                    blkio_device_write_bps_path,
                    blkio_device_write_bps_rate,
                    blkio_device_read_iops_path,
                    blkio_device_read_iops_rate,
                    blkio_device_write_iops_path,
                    blkio_device_write_iops_rate,
                    securityopt,
                    masked_paths,
                    read_only_paths,
                } = create_container.as_ref();

                let container_id = SqlUuid::new(id);
                let image_id = SqlUuid::new(image_id);

                let mut container = Container::try_from(create_container.as_ref())
                    .map_err(|err| HandleError::Application(err.into()))?;

                if Container::exists(&container_id).get_result(writer)? {
                    debug!("container already exists");

                    return Ok(());
                }

                let image_exists: bool = Image::exists(&image_id).get_result(writer)?;

                if !image_exists {
                    debug!("image is missing, storing image_id into container_missing_images");

                    container.image_id.take();
                }

                insert_or_ignore_into(containers::table)
                    .values(&container)
                    .execute(writer)?;

                if !image_exists {
                    insert_or_ignore_into(container_missing_images::table)
                        .values(ContainerMissingImage {
                            container_id,
                            image_id,
                        })
                        .execute(writer)?;
                }

                for value in map_opt_vec(env) {
                    insert_or_ignore_into(container_env::table)
                        .values(ContainerEnv {
                            container_id,
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                for value in map_opt_vec(extra_hosts) {
                    insert_or_ignore_into(container_extra_hosts::table)
                        .values(ContainerExtraHost {
                            container_id,
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                for value in map_opt_vec(binds) {
                    insert_or_ignore_into(container_binds::table)
                        .values(ContainerBind {
                            container_id,
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                for (port, idx) in map_opt_vec(port_bindings).iter().zip(0..) {
                    let bind = parse_port_binding(port.as_str())
                        .map_err(|err| HandleError::Application(err.into()))?;

                    insert_or_ignore_into(container_port_bindings::table)
                        .values(ContainerPortBind {
                            container_id,
                            port: bind.port_proto.into(),
                            idx,
                            host_ip: bind.host.host_ip.map(Into::into),
                            host_port: bind.host.host_port.map(HostPort),
                        })
                        .execute(writer)?;
                }

                for value in map_opt_vec(cap_add) {
                    insert_or_ignore_into(container_add_capabilities::table)
                        .values(ContainerAddCapability {
                            container_id,
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                for value in map_opt_vec(cap_drop) {
                    insert_or_ignore_into(container_drop_capabilities::table)
                        .values(ContainerDropCapability {
                            container_id,
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                let storage_opt = map_opt_vec(storage_opt_keys)
                    .iter()
                    .zip(map_opt_vec(storage_opt_values));
                for (name, value) in storage_opt {
                    insert_or_ignore_into(container_storage_options::table)
                        .values(ContainerStorageOptions {
                            container_id,
                            name: name.into(),
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                let tmpfs = map_opt_vec(tmpfs_paths)
                    .iter()
                    .zip(map_opt_vec(tmpfs_options));
                for (path, options) in tmpfs {
                    insert_or_ignore_into(container_tmpfs::table)
                        .values(ContainerTmpfs {
                            container_id,
                            path: path.into(),
                            options: options.into(),
                        })
                        .execute(writer)?;
                }

                for (cmd, idx) in map_opt_vec(cmd).iter().zip(0..) {
                    insert_or_ignore_into(container_cmds::table)
                        .values(ContainerCmd {
                            container_id,
                            idx,
                            cmd: cmd.into(),
                        })
                        .execute(writer)?;
                }

                for (cmd, idx) in map_opt_vec(health_check_test).iter().zip(0..) {
                    insert_or_ignore_into(container_healthcheck_test::table)
                        .values(ContainerHealthcheckTest {
                            container_id,
                            idx,
                            cmd: cmd.into(),
                        })
                        .execute(writer)?;
                }

                for (cmd, idx) in map_opt_vec(entrypoint).iter().zip(0..) {
                    insert_or_ignore_into(container_entrypoints::table)
                        .values(ContainerEntrypoint {
                            container_id,
                            idx,
                            cmd: cmd.into(),
                        })
                        .execute(writer)?;
                }

                let labels = map_opt_vec(labels_keys)
                    .iter()
                    .zip(map_opt_vec(labels_values));
                for (key, value) in labels {
                    insert_or_ignore_into(container_labels::table)
                        .values(ContainerLabel {
                            container_id,
                            key: key.into(),
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                for port in map_opt_vec(exposed_ports) {
                    insert_or_ignore_into(container_exposed_ports::table)
                        .values(ContainerExposedPort {
                            container_id,
                            port: port.into(),
                        })
                        .execute(writer)?;
                }

                for rule in map_opt_vec(device_cgroup_rules) {
                    insert_or_ignore_into(container_device_cgroup_rules::table)
                        .values(ContainerDeviceCgroupRule {
                            container_id,
                            rule: rule.into(),
                        })
                        .execute(writer)?;
                }

                let ulimits = map_opt_vec(ulimits_name)
                    .iter()
                    .zip(map_opt_vec(ulimits_soft))
                    .zip(map_opt_vec(ulimits_hard));
                for ((name, soft), hard) in ulimits {
                    insert_or_ignore_into(container_ulimits::table)
                        .values(ContainerUlimit {
                            container_id,
                            name: name.into(),
                            soft: *soft,
                            hard: *hard,
                        })
                        .execute(writer)?;
                }

                for dns in map_opt_vec(dns) {
                    insert_or_ignore_into(container_dns::table)
                        .values(ContainerDns {
                            container_id,
                            dns: dns.into(),
                        })
                        .execute(writer)?;
                }

                for dns_option in map_opt_vec(dns_options) {
                    insert_or_ignore_into(container_dns_options::table)
                        .values(ContainerDnsOption {
                            container_id,
                            dns_option: dns_option.into(),
                        })
                        .execute(writer)?;
                }

                for dns_search in map_opt_vec(dns_search) {
                    insert_or_ignore_into(container_dns_search::table)
                        .values(ContainerDnsSearch {
                            container_id,
                            dns_search: dns_search.into(),
                        })
                        .execute(writer)?;
                }

                for group in map_opt_vec(group_add) {
                    insert_or_ignore_into(container_group_add::table)
                        .values(ContainerGroupAdd {
                            container_id,
                            group_add: group.into(),
                        })
                        .execute(writer)?;
                }

                let sysctls = map_opt_vec(sysctls_keys)
                    .iter()
                    .zip(map_opt_vec(sysctls_values));
                for (key, value) in sysctls {
                    insert_or_ignore_into(container_sysctls::table)
                        .values(ContainerSysctl {
                            container_id,
                            key: key.into(),
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                let log_config = map_opt_vec(log_config_keys)
                    .iter()
                    .zip(map_opt_vec(log_config_values));
                for (key, value) in log_config {
                    insert_or_ignore_into(container_log_config::table)
                        .values(ContainerLogConfig {
                            container_id,
                            key: key.into(),
                            value: value.into(),
                        })
                        .execute(writer)?;
                }

                let blkio_device_weight = map_opt_vec(blkio_weight_device_path)
                    .iter()
                    .zip(map_opt_vec(blkio_weight_device_weight));
                for (path, weight) in blkio_device_weight {
                    insert_or_ignore_into(container_blkio_weight_device::table)
                        .values(ContainerBlkioWeightDevice {
                            container_id,
                            path: path.into(),
                            weight: *weight,
                        })
                        .execute(writer)?;
                }

                let blkio_device_read_bps = map_opt_vec(blkio_device_read_bps_path)
                    .iter()
                    .zip(map_opt_vec(blkio_device_read_bps_rate));
                for (path, rate) in blkio_device_read_bps {
                    insert_or_ignore_into(container_blkio_device_read_bps::table)
                        .values(ContainerBlkioDeviceReadBps {
                            container_id,
                            path: path.into(),
                            rate: *rate,
                        })
                        .execute(writer)?;
                }

                let blkio_device_write_bps = map_opt_vec(blkio_device_write_bps_path)
                    .iter()
                    .zip(map_opt_vec(blkio_device_write_bps_rate));
                for (path, rate) in blkio_device_write_bps {
                    insert_or_ignore_into(container_blkio_device_write_bps::table)
                        .values(ContainerBlkioDeviceWriteBps {
                            container_id,
                            path: path.into(),
                            rate: *rate,
                        })
                        .execute(writer)?;
                }

                let blkio_device_read_iops = map_opt_vec(blkio_device_read_iops_path)
                    .iter()
                    .zip(map_opt_vec(blkio_device_read_iops_rate));
                for (path, rate) in blkio_device_read_iops {
                    insert_or_ignore_into(container_blkio_device_read_iops::table)
                        .values(ContainerBlkioDeviceReadIops {
                            container_id,
                            path: path.into(),
                            rate: *rate,
                        })
                        .execute(writer)?;
                }

                let blkio_device_write_iops = map_opt_vec(blkio_device_write_iops_path)
                    .iter()
                    .zip(map_opt_vec(blkio_device_write_iops_rate));
                for (path, rate) in blkio_device_write_iops {
                    insert_or_ignore_into(container_blkio_device_write_iops::table)
                        .values(ContainerBlkioDeviceWriteIops {
                            container_id,
                            path: path.into(),
                            rate: *rate,
                        })
                        .execute(writer)?;
                }

                for opt in map_opt_vec(securityopt) {
                    insert_or_ignore_into(container_securityopts::table)
                        .values(ContainerSecurityopt {
                            container_id,
                            opt: opt.into(),
                        })
                        .execute(writer)?;
                }

                for masked_path in map_opt_vec(masked_paths) {
                    insert_or_ignore_into(container_masked_paths::table)
                        .values(ContainerMaskedPath {
                            container_id,
                            path: masked_path.into(),
                        })
                        .execute(writer)?;
                }

                for read_only_path in map_opt_vec(read_only_paths) {
                    insert_or_ignore_into(container_readonly_paths::table)
                        .values(ContainerReadonlyPath {
                            container_id,
                            path: read_only_path.into(),
                        })
                        .execute(writer)?;
                }

                for network_id in map_uuids_vec_to_sql(network_ids) {
                    let network_exists: bool = Network::exists(&network_id).get_result(writer)?;

                    if !network_exists {
                        insert_or_ignore_into(container_missing_networks::table)
                            .values(ContainerMissingNetwork {
                                container_id,
                                network_id,
                            })
                            .execute(writer)?;
                    } else {
                        insert_or_ignore_into(container_networks::table)
                            .values(ContainerNetwork {
                                container_id,
                                network_id,
                            })
                            .execute(writer)?;
                    }
                }

                for volume_id in map_uuids_vec_to_sql(volume_ids) {
                    let volume_exists: bool = Volume::exists(&volume_id).get_result(writer)?;

                    if !volume_exists {
                        insert_or_ignore_into(container_missing_volumes::table)
                            .values(ContainerMissingVolume {
                                container_id,
                                volume_id,
                            })
                            .execute(writer)?;
                    } else {
                        insert_or_ignore_into(container_volumes::table)
                            .values(ContainerVolume {
                                container_id,
                                volume_id,
                            })
                            .execute(writer)?;
                    }
                }

                for device_mapping_id in map_uuids_vec_to_sql(device_mapping_ids) {
                    let device_mapping_exists: bool =
                        DeviceMapping::exists(&device_mapping_id).get_result(writer)?;

                    if !device_mapping_exists {
                        insert_or_ignore_into(container_missing_device_mappings::table)
                            .values(ContainerMissingDeviceMapping {
                                container_id,
                                device_mapping_id,
                            })
                            .execute(writer)?;
                    } else {
                        insert_or_ignore_into(container_device_mappings::table)
                            .values(ContainerDeviceMapping {
                                container_id,
                                device_mapping_id,
                            })
                            .execute(writer)?;
                    }
                }

                for device_request_id in map_uuids_vec_to_sql(device_request_ids) {
                    let device_request_exists: bool =
                        DeviceRequest::exists(&device_request_id).get_result(writer)?;

                    if !device_request_exists {
                        insert_or_ignore_into(container_missing_device_requests::table)
                            .values(ContainerMissingDeviceRequest {
                                container_id,
                                device_request_id,
                            })
                            .execute(writer)?;
                    } else {
                        insert_or_ignore_into(container_device_requests::table)
                            .values(ContainerDeviceRequest {
                                container_id,
                                device_request_id,
                            })
                            .execute(writer)?;
                    }
                }

                // Update deployment missing container
                insert_or_ignore_into(deployment_containers::table)
                    .values(DeploymentMissingContainer::find_by_container(&container_id))
                    .execute(writer)?;

                delete(DeploymentMissingContainer::find_by_container(&container_id))
                    .execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }
}

impl<'a> TryFrom<&'a CreateContainer> for container::Container<'a> {
    type Error = StoreError;

    fn try_from(value: &'a CreateContainer) -> Result<Self, Self::Error> {
        let CreateContainer {
            id,
            deployment_id,
            image_id,
            hostname,
            restart_policy,
            network_mode,
            cpu_period,
            cpu_quota,
            cpu_realtime_period,
            cpu_realtime_runtime,
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
            restart_policy_maximum_retry_count,
            cpu_shares,
            auto_remove,
            cgroupns_mode,
            ipc_mode,
            oom_score_adj,
            userns_mode,
            shm_size,
            runtime,
            log_type,
            blkio_weight,
            pid_mode,
            ..
        } = value;

        debug!(?deployment_id);

        let cgroupns_mode = cgroupns_mode
            .as_deref()
            .map(CgroupnsMode::from_str)
            .transpose()
            .map_err(|ctx| StoreError::Conversion { ctx })?;

        let restart_policy = restart_policy
            .as_deref()
            .map(ContainerRestartPolicy::from_str)
            .transpose()
            .map_err(|ctx| StoreError::Conversion { ctx })?;

        let memory_swappiness = memory_swappiness
            .map(|swappiness| match i16::try_from(swappiness) {
                Ok(swappiness @ 0..100) => Ok(swappiness),
                _ => Err(StoreError::Conversion {
                    ctx: format!("invalid memory swappiness value {swappiness}"),
                }),
            })
            .transpose()?;

        // Make sure both are unset
        match (&cpu_period, &cpu_quota) {
            (None, None) | (Some(_), Some(_)) => {}
            (None, Some(_)) => {
                warn!("cpu period missing, while cpu quota is set");
            }
            (Some(_), None) => {
                warn!("cpu period was set, while cpu quota is missing");
            }
        }

        if restart_policy_maximum_retry_count.is_some()
            && restart_policy.is_none_or(|policy| policy != ContainerRestartPolicy::OnFailure)
        {
            warn!(
                "reastart policy maximum retry count must be used with a restart pocily of 'on-failure'"
            );
        }

        if auto_remove.is_some_and(|rm| rm)
            && restart_policy.is_some_and(|policy| {
                !matches!(
                    policy,
                    ContainerRestartPolicy::Empty | ContainerRestartPolicy::No
                )
            })
        {
            warn!("auto remove has no effect with a restart policy");
        }

        if let Some(shm) = *shm_size
            && shm < 0
        {
            return Err(StoreError::Conversion {
                ctx: format!("shm size must be greater than zero, but got {shm}"),
            });
        }

        let blkio_weight = match *blkio_weight {
            Some(blkio_weight @ 0..1000) => Some(blkio_weight as i16),
            None => None,
            Some(other) => {
                return Err(StoreError::Conversion {
                    ctx: format!("blkio weight must be between 0 and 1000, but got {other}"),
                });
            }
        };

        let container_id = SqlUuid::new(id);

        Ok(container::Container {
            id: container_id,
            local_id: None,
            image_id: Some(SqlUuid::new(image_id)),
            status: ContainerStatus::default(),
            network_mode: network_mode.as_ref().map(Cow::from),
            hostname: hostname.as_ref().map(Cow::from),
            restart_policy,
            cpu_period: *cpu_period,
            cpu_quota: *cpu_quota,
            cpu_realtime_period: *cpu_realtime_period,
            cpu_realtime_runtime: *cpu_realtime_runtime,
            memory: *memory,
            memory_reservation: *memory_reservation,
            memory_swap: *memory_swap,
            memory_swappiness,
            volume_driver: volume_driver.as_ref().map(Cow::from),
            read_only_rootfs: *read_only_rootfs,
            privileged: *privileged,
            domainname: domainname.as_ref().map(Cow::from),
            user: user.as_ref().map(Cow::from),
            health_check_interval: *health_check_interval,
            health_check_timeout: *health_check_timeout,
            health_check_retries: *health_check_retries,
            restart_policy_maximum_retry_count: *restart_policy_maximum_retry_count,
            cpu_shares: *cpu_shares,
            health_check_start_period: *health_check_start_period,
            health_check_start_interval: *health_check_start_interval,
            working_dir: working_dir.as_ref().map(Cow::from),
            network_disabled: *network_disabled,
            stop_signal: stop_signal.as_ref().map(Cow::from),
            stop_timeout: *stop_timeout,
            auto_remove: *auto_remove,
            cgroupns_mode,
            ipc_mode: ipc_mode.as_ref().map(Cow::from),
            oom_score_adj: *oom_score_adj,
            userns_mode: userns_mode.as_ref().map(Cow::from),
            shm_size: *shm_size,
            log_type: log_type.as_ref().map(Cow::from),
            blkio_weight,
            pid_mode: pid_mode.as_ref().map(Cow::from),
            runtime: runtime.as_ref().map(Cow::from),
        })
    }
}

fn map_uuids_vec_to_sql(value: &Option<VecReqUuid>) -> impl Iterator<Item = SqlUuid> {
    match value {
        Some(v) => v.as_slice(),
        None => &[],
    }
    .iter()
    .map(SqlUuid::new)
}

fn map_opt_vec<T>(value: &Option<Vec<T>>) -> &[T] {
    match value {
        Some(v) => v.as_slice(),
        None => &[],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::container::tests::create_container_resource;
    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::image::CreateImage;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;
    use crate::store::container::tests::find_container;

    use super::*;

    pub(crate) fn stored_container_full(id: Uuid, image: &CreateImage) -> Container<'static> {
        Container {
            id: SqlUuid::new(id),
            local_id: None,
            image_id: Some(SqlUuid::new(image.id.0)),
            status: ContainerStatus::default(),
            network_mode: Some(Cow::Borrowed("bridge")),
            hostname: Some(Cow::Borrowed("database")),
            restart_policy: Some(ContainerRestartPolicy::UnlessStopped),
            restart_policy_maximum_retry_count: Some(4),
            cpu_period: Some(1000),
            cpu_quota: Some(100),
            cpu_realtime_period: Some(1000),
            cpu_realtime_runtime: Some(100),
            cpu_shares: Some(100),
            memory: Some(4096),
            memory_reservation: Some(1024),
            memory_swap: Some(8192),
            memory_swappiness: Some(50),
            volume_driver: Some(Cow::Borrowed("local")),
            read_only_rootfs: Some(true),
            privileged: Some(false),
            domainname: Some(Cow::Borrowed("database.host")),
            user: Some(Cow::Borrowed("pg")),
            health_check_interval: Some(42_000),
            health_check_timeout: Some(6_000),
            health_check_retries: Some(3),
            health_check_start_period: Some(10_000),
            health_check_start_interval: Some(10_000),
            working_dir: Some(Cow::Borrowed("/app-data")),
            network_disabled: Some(false),
            stop_signal: Some(Cow::Borrowed("SIGSTOP")),
            stop_timeout: Some(32),
            auto_remove: Some(false),
            cgroupns_mode: Some(CgroupnsMode::Private),
            ipc_mode: Some(Cow::Borrowed("shareable")),
            oom_score_adj: Some(500),
            userns_mode: Some(Cow::Borrowed("host")),
            shm_size: Some(67_108_864),
            log_type: Some(Cow::Borrowed("json-file")),
            blkio_weight: Some(500),
            pid_mode: Some(Cow::Borrowed("host")),
            runtime: Some(Cow::Borrowed("runc")),
        }
    }

    #[tokio::test]
    async fn should_create_full() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let container_id = container.id.0;

        store.create_image(image.clone()).await.unwrap();
        store.create_volume(volume).await.unwrap();
        store.create_network(network).await.unwrap();
        store.create_device_mapping(device_mapping).await.unwrap();
        store.create_device_request(device_request).await.unwrap();

        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();

        let exp = stored_container_full(container_id, &image);
        let res = find_container(&store, container.id.0).await.unwrap();

        assert_eq!(res, exp);

        let exp = create_container_resource(&container);

        let res = store.find_container(container.id.0).await.unwrap().unwrap();
        assert_eq!(res, exp);
    }
}
