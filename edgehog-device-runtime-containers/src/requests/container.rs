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

//! Create image request

use astarte_device_sdk::FromEvent;
use tracing::{instrument, trace};

use crate::{container::Binding, requests::BindingError};

use super::{ReqUuid, VecReqUuid};

/// Request to pull a Docker Container.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord, Default)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateContainerRequest",
    path = "/container",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateContainer {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) image_id: ReqUuid,
    pub(crate) network_ids: Option<VecReqUuid>,
    pub(crate) volume_ids: Option<VecReqUuid>,
    pub(crate) device_mapping_ids: Option<VecReqUuid>,
    pub(crate) device_request_ids: Option<VecReqUuid>,
    pub(crate) hostname: Option<String>,
    pub(crate) restart_policy: Option<String>,
    pub(crate) env: Option<Vec<String>>,
    pub(crate) binds: Option<Vec<String>>,
    pub(crate) network_mode: Option<String>,
    pub(crate) port_bindings: Option<Vec<String>>,
    pub(crate) extra_hosts: Option<Vec<String>>,
    pub(crate) cap_add: Option<Vec<String>>,
    pub(crate) cap_drop: Option<Vec<String>>,
    pub(crate) cpu_period: Option<i64>,
    pub(crate) cpu_quota: Option<i64>,
    pub(crate) cpu_realtime_period: Option<i64>,
    pub(crate) cpu_realtime_runtime: Option<i64>,
    pub(crate) memory: Option<i64>,
    pub(crate) memory_reservation: Option<i64>,
    pub(crate) memory_swap: Option<i64>,
    pub(crate) memory_swappiness: Option<i32>,
    pub(crate) volume_driver: Option<String>,
    pub(crate) storage_opt_keys: Option<Vec<String>>,
    pub(crate) storage_opt_values: Option<Vec<String>>,
    pub(crate) read_only_rootfs: Option<bool>,
    pub(crate) tmpfs_paths: Option<Vec<String>>,
    pub(crate) tmpfs_options: Option<Vec<String>>,
    pub(crate) privileged: Option<bool>,
    pub(crate) domainname: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) cmd: Option<Vec<String>>,
    pub(crate) health_check_test: Option<Vec<String>>,
    pub(crate) health_check_interval: Option<i64>,
    pub(crate) health_check_timeout: Option<i64>,
    pub(crate) health_check_retries: Option<i32>,
    pub(crate) health_check_start_period: Option<i64>,
    pub(crate) health_check_start_interval: Option<i64>,
    pub(crate) working_dir: Option<String>,
    pub(crate) entrypoint: Option<Vec<String>>,
    pub(crate) network_disabled: Option<bool>,
    pub(crate) labels_keys: Option<Vec<String>>,
    pub(crate) labels_values: Option<Vec<String>>,
    pub(crate) stop_signal: Option<String>,
    pub(crate) stop_timeout: Option<i32>,
    pub(crate) restart_policy_maximum_retry_count: Option<i32>,
    pub(crate) exposed_ports: Option<Vec<String>>,
    pub(crate) cpu_shares: Option<i32>,
    pub(crate) device_cgroup_rules: Option<Vec<String>>,
    pub(crate) ulimits_name: Option<Vec<String>>,
    pub(crate) ulimits_soft: Option<Vec<i32>>,
    pub(crate) ulimits_hard: Option<Vec<i32>>,
    pub(crate) auto_remove: Option<bool>,
    pub(crate) cgroupns_mode: Option<String>,
    pub(crate) dns: Option<Vec<String>>,
    pub(crate) dns_options: Option<Vec<String>>,
    pub(crate) dns_search: Option<Vec<String>>,
    pub(crate) group_add: Option<Vec<String>>,
    pub(crate) ipc_mode: Option<String>,
    pub(crate) oom_score_adj: Option<i32>,
    pub(crate) userns_mode: Option<String>,
    pub(crate) sysctls_keys: Option<Vec<String>>,
    pub(crate) sysctls_values: Option<Vec<String>>,
    pub(crate) shm_size: Option<i64>,
    pub(crate) runtime: Option<String>,
    pub(crate) log_type: Option<String>,
    pub(crate) log_config_keys: Option<Vec<String>>,
    pub(crate) log_config_values: Option<Vec<String>>,
    pub(crate) blkio_weight: Option<i32>,
    pub(crate) blkio_weight_device_path: Option<Vec<String>>,
    pub(crate) blkio_weight_device_weight: Option<Vec<i32>>,
    pub(crate) blkio_device_read_bps_path: Option<Vec<String>>,
    pub(crate) blkio_device_read_bps_rate: Option<Vec<i64>>,
    pub(crate) blkio_device_write_bps_path: Option<Vec<String>>,
    pub(crate) blkio_device_write_bps_rate: Option<Vec<i64>>,
    pub(crate) blkio_device_read_iops_path: Option<Vec<String>>,
    pub(crate) blkio_device_read_iops_rate: Option<Vec<i64>>,
    pub(crate) blkio_device_write_iops_path: Option<Vec<String>>,
    pub(crate) blkio_device_write_iops_rate: Option<Vec<i64>>,
    pub(crate) securityopt: Option<Vec<String>>,
    pub(crate) pid_mode: Option<String>,
    pub(crate) masked_paths: Option<Vec<String>>,
    pub(crate) read_only_paths: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedBind<'a> {
    pub(crate) port_proto: &'a str,
    pub(crate) port: u16,
    pub(crate) proto: Option<&'a str>,
    pub(crate) host: Binding<&'a str>,
}

impl<'a> ParsedBind<'a> {
    fn new(
        port_proto: &'a str,
        port: u16,
        proto: Option<&'a str>,
        host_ip: Option<&'a str>,
        host_port: Option<u16>,
    ) -> Self {
        debug_assert_eq!(
            port_proto,
            if let Some(proto) = proto {
                format!("{port}/{proto}")
            } else {
                port.to_string()
            }
        );

        Self {
            port_proto,
            port,
            proto,
            host: Binding { host_ip, host_port },
        }
    }
}

/// Parses a binding in the form
///
/// ```plaintext
/// [ip:[hostPort:]]containerPort[/protocol]
/// ```
#[instrument]
pub(crate) fn parse_port_binding(input: &str) -> Result<ParsedBind<'_>, BindingError> {
    let (host_ip, host_port, rest) = parse_host_ip_port(input)?;

    let (container_port, protocol) = rest.split_once('/').map_or_else(
        || {
            trace!("container port {rest}");

            (rest, None)
        },
        |(port, proto)| {
            trace!("container port {port} and protocol {proto}");

            (port, Some(proto))
        },
    );

    let container_port = container_port.parse().map_err(|err| BindingError::Port {
        binding: "container",
        value: container_port.to_string(),
        source: err,
    })?;

    Ok(ParsedBind::new(
        rest,
        container_port,
        protocol,
        host_ip,
        host_port,
    ))
}

#[instrument]
fn parse_host_ip_port(input: &str) -> Result<(Option<&str>, Option<u16>, &str), BindingError> {
    let Some((ip_or_port, rest)) = input.split_once(':') else {
        trace!("missing host ip or port, returning rest: {input}");

        return Ok((None, None, input));
    };

    match rest.split_once(':') {
        Some((port, rest)) => {
            let port: u16 = port.parse().map_err(|err| BindingError::Port {
                binding: "host",
                value: port.to_string(),
                source: err,
            })?;

            trace!("found ip {ip_or_port} and port {port}");

            Ok((Some(ip_or_port), Some(port), rest))
        }
        None => {
            // Try to parse the ip as port
            if let Ok(port) = ip_or_port.parse::<u16>() {
                trace!("found port {port}");

                Ok((None, Some(port), rest))
            } else {
                trace!("found ip {ip_or_port}");
                Ok((Some(ip_or_port), None, rest))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use crate::requests::device_mapping::CreateDeviceMapping;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::CreateDeviceRequest;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::image::CreateImage;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::CreateNetwork;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::CreateVolume;
    use crate::requests::volume::tests::create_volume_req;

    use super::*;

    pub(crate) fn create_container_req(
        deployment_id: Uuid,
        image: &CreateImage,
        volume: &CreateVolume,
        network: &CreateNetwork,
        device_mapping: &CreateDeviceMapping,
        device_request: &CreateDeviceRequest,
    ) -> CreateContainer {
        CreateContainer {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            image_id: image.id,
            network_ids: Some(VecReqUuid(vec![network.id])),
            volume_ids: Some(VecReqUuid(vec![volume.id])),
            device_mapping_ids: Some(VecReqUuid(vec![device_mapping.id])),
            device_request_ids: Some(VecReqUuid(vec![device_request.id])),
            hostname: Some("database".to_string()),
            restart_policy: Some("unless-stopped".to_string()),
            env: Some(
                ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                    .map(str::to_string)
                    .to_vec(),
            ),
            binds: Some(vec!["/var/lib/postgres:/data".to_string()]),
            network_mode: Some("bridge".to_string()),
            port_bindings: Some(vec!["5432:5432".to_string()]),
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
            storage_opt_keys: Some(vec!["size".to_string()]),
            storage_opt_values: Some(vec!["1024k".to_string()]),
            read_only_rootfs: Some(true),
            tmpfs_paths: Some(vec!["/run".to_string()]),
            tmpfs_options: Some(vec!["rw,noexec,nosuid,size=65536k".to_string()]),
            privileged: Some(false),
            domainname: Some("database.host".to_string()),
            user: Some("pg".to_string()),
            cmd: Some(["-u=postgres", "-d=test"].map(str::to_string).to_vec()),
            health_check_test: Some(
                ["pg-is-ready", "-u=postgres", "-d=test"]
                    .map(str::to_string)
                    .to_vec(),
            ),
            health_check_interval: Some(42_000),
            health_check_timeout: Some(6_000),
            health_check_retries: Some(3),
            health_check_start_period: Some(10_000),
            health_check_start_interval: Some(10_000),
            working_dir: Some("/app-data".to_string()),
            entrypoint: Some(["CMD-SHELL", "/entrypoint.sh"].map(str::to_string).to_vec()),
            network_disabled: Some(false),
            labels_keys: Some(["lab_1", "lab_2"].map(str::to_string).to_vec()),
            labels_values: Some(["foo", "bar"].map(str::to_string).to_vec()),
            stop_signal: Some("SIGSTOP".to_string()),
            stop_timeout: Some(32),
            restart_policy_maximum_retry_count: Some(4),
            exposed_ports: Some(["90/tcp", "53/udp", "9000"].map(str::to_string).to_vec()),
            cpu_shares: Some(100),
            device_cgroup_rules: Some(vec!["c 1:3 mr".to_string()]),
            ulimits_name: Some(vec!["nofile".to_string()]),
            ulimits_soft: Some(vec![1024]),
            ulimits_hard: Some(vec![2048]),
            auto_remove: Some(false),
            cgroupns_mode: Some("private".to_string()),
            dns: Some(vec!["8.8.8.8".to_string()]),
            dns_options: Some(vec!["timeout:3".to_string()]),
            dns_search: Some(vec!["example.com".to_string()]),
            group_add: Some(vec!["dialout".to_string()]),
            ipc_mode: Some("shareable".to_string()),
            oom_score_adj: Some(500),
            userns_mode: Some("host".to_string()),
            sysctls_keys: Some(vec!["net.ipv4.ip_forward".to_string()]),
            sysctls_values: Some(vec!["1".to_string()]),
            shm_size: Some(67_108_864), // 64 MB
            runtime: Some("runc".to_string()),
            log_type: Some("json-file".to_string()),
            log_config_keys: Some(vec!["max-size".to_string(), "max-file".to_string()]),
            log_config_values: Some(vec!["10m".to_string(), "3".to_string()]),
            blkio_weight: Some(500),
            blkio_weight_device_path: Some(vec!["/dev/sda".to_string()]),
            blkio_weight_device_weight: Some(vec![500]),
            blkio_device_read_bps_path: Some(vec!["/dev/sda".to_string()]),
            blkio_device_read_bps_rate: Some(vec![1_048_576]), // 1 MB/s
            blkio_device_write_bps_path: Some(vec!["/dev/sda".to_string()]),
            blkio_device_write_bps_rate: Some(vec![1_048_576]),
            blkio_device_read_iops_path: Some(vec!["/dev/sda".to_string()]),
            blkio_device_read_iops_rate: Some(vec![1000]),
            blkio_device_write_iops_path: Some(vec!["/dev/sda".to_string()]),
            blkio_device_write_iops_rate: Some(vec![1000]),
            securityopt: Some(vec!["no-new-privileges".to_string()]),
            pid_mode: Some("host".to_string()),
            masked_paths: Some(vec!["/proc/kcore".to_string()]),
            read_only_paths: Some(vec!["/proc/sys".to_string()]),
        }
    }

    pub fn create_container_request_event(container: &CreateContainer) -> DeviceEvent {
        let network_ids = container
            .network_ids
            .iter()
            .flat_map(|i| i.iter().map(|id| id.to_string()))
            .collect();
        let volume_ids = container
            .volume_ids
            .iter()
            .flat_map(|i| i.iter().map(|id| id.to_string()))
            .collect();
        let device_mapping_ids = container
            .device_mapping_ids
            .iter()
            .flat_map(|i| i.iter().map(|id| id.to_string()))
            .collect();
        let device_request_ids = container
            .device_request_ids
            .iter()
            .flat_map(|i| i.iter().map(|id| id.to_string()))
            .collect();

        let fields = [
            ("id", AstarteData::String(container.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(container.deployment_id.to_string()),
            ),
            (
                "imageId",
                AstarteData::String(container.image_id.to_string()),
            ),
            ("networkIds", AstarteData::StringArray(network_ids)),
            ("volumeIds", AstarteData::StringArray(volume_ids)),
            (
                "deviceMappingIds",
                AstarteData::StringArray(device_mapping_ids),
            ),
            (
                "deviceRequestIds",
                AstarteData::StringArray(device_request_ids),
            ),
            ("hostname", AstarteData::String("database".to_string())),
            (
                "restartPolicy",
                AstarteData::String("unless-stopped".to_string()),
            ),
            (
                "env",
                AstarteData::StringArray(
                    ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                        .map(str::to_string)
                        .to_vec(),
                ),
            ),
            (
                "binds",
                AstarteData::StringArray(vec!["/var/lib/postgres:/data".to_string()]),
            ),
            ("networkMode", AstarteData::String("bridge".to_string())),
            (
                "portBindings",
                AstarteData::StringArray(vec!["5432:5432".to_string()]),
            ),
            (
                "extraHosts",
                AstarteData::StringArray(vec!["host.docker.internal:host-gateway".to_string()]),
            ),
            (
                "capAdd",
                AstarteData::StringArray(vec!["CAP_CHOWN".to_string()]),
            ),
            (
                "capDrop",
                AstarteData::StringArray(vec!["CAP_KILL".to_string()]),
            ),
            ("cpuPeriod", AstarteData::Integer(1000)),
            ("cpuQuota", AstarteData::Integer(100)),
            ("cpuRealtimePeriod", AstarteData::Integer(1000)),
            ("cpuRealtimeRuntime", AstarteData::Integer(100)),
            ("memory", AstarteData::Integer(4096)),
            ("memoryReservation", AstarteData::Integer(1024)),
            ("memorySwap", AstarteData::Integer(8192)),
            ("memorySwappiness", AstarteData::Integer(50)),
            ("volumeDriver", AstarteData::String("local".to_string())),
            (
                "storageOptKeys",
                AstarteData::StringArray(vec!["size".to_string()]),
            ),
            (
                "storageOptValues",
                AstarteData::StringArray(vec!["1024k".to_string()]),
            ),
            ("readOnlyRootfs", AstarteData::Boolean(true)),
            (
                "tmpfsPaths",
                AstarteData::StringArray(vec!["/run".to_string()]),
            ),
            (
                "tmpfsOptions",
                AstarteData::StringArray(vec!["rw,noexec,nosuid,size=65536k".to_string()]),
            ),
            ("privileged", AstarteData::Boolean(false)),
            (
                "domainname",
                AstarteData::String("database.host".to_string()),
            ),
            ("user", AstarteData::String("pg".to_string())),
            (
                "cmd",
                AstarteData::StringArray(["-u=postgres", "-d=test"].map(str::to_string).to_vec()),
            ),
            (
                "healthCheckTest",
                AstarteData::StringArray(
                    ["pg-is-ready", "-u=postgres", "-d=test"]
                        .map(str::to_string)
                        .to_vec(),
                ),
            ),
            ("healthCheckInterval", AstarteData::Integer(42_000)),
            ("healthCheckTimeout", AstarteData::Integer(6_000)),
            ("healthCheckRetries", AstarteData::Integer(3)),
            ("healthCheckStartPeriod", AstarteData::Integer(10_000)),
            ("healthCheckStartInterval", AstarteData::Integer(10_000)),
            ("workingDir", AstarteData::String("/app-data".to_string())),
            (
                "entrypoint",
                AstarteData::StringArray(
                    ["CMD-SHELL", "/entrypoint.sh"].map(str::to_string).to_vec(),
                ),
            ),
            ("networkDisabled", AstarteData::Boolean(false)),
            (
                "labelsKeys",
                AstarteData::StringArray(["lab_1", "lab_2"].map(str::to_string).to_vec()),
            ),
            (
                "labelsValues",
                AstarteData::StringArray(["foo", "bar"].map(str::to_string).to_vec()),
            ),
            ("stopSignal", AstarteData::String("SIGSTOP".to_string())),
            ("stopTimeout", AstarteData::Integer(32)),
            ("restartPolicyMaximumRetryCount", AstarteData::Integer(4)),
            (
                "exposedPorts",
                AstarteData::StringArray(["90/tcp", "53/udp", "9000"].map(str::to_string).to_vec()),
            ),
            ("cpuShares", AstarteData::Integer(100)),
            (
                "deviceCgroupRules",
                AstarteData::StringArray(vec!["c 1:3 mr".to_string()]),
            ),
            (
                "ulimitsName",
                AstarteData::StringArray(vec!["nofile".to_string()]),
            ),
            ("ulimitsSoft", AstarteData::IntegerArray(vec![1024])),
            ("ulimitsHard", AstarteData::IntegerArray(vec![2048])),
            ("autoRemove", AstarteData::Boolean(false)),
            ("cgroupnsMode", AstarteData::String("private".to_string())),
            ("dns", AstarteData::StringArray(vec!["8.8.8.8".to_string()])),
            (
                "dnsOptions",
                AstarteData::StringArray(vec!["timeout:3".to_string()]),
            ),
            (
                "dnsSearch",
                AstarteData::StringArray(vec!["example.com".to_string()]),
            ),
            (
                "groupAdd",
                AstarteData::StringArray(vec!["dialout".to_string()]),
            ),
            ("ipcMode", AstarteData::String("shareable".to_string())),
            ("oomScoreAdj", AstarteData::Integer(500)),
            ("usernsMode", AstarteData::String("host".to_string())),
            (
                "sysctlsKeys",
                AstarteData::StringArray(vec!["net.ipv4.ip_forward".to_string()]),
            ),
            (
                "sysctlsValues",
                AstarteData::StringArray(vec!["1".to_string()]),
            ),
            ("shmSize", AstarteData::Integer(67_108_864)),
            ("runtime", AstarteData::String("runc".to_string())),
            ("logType", AstarteData::String("json-file".to_string())),
            (
                "logConfigKeys",
                AstarteData::StringArray(vec!["max-size".to_string(), "max-file".to_string()]),
            ),
            (
                "logConfigValues",
                AstarteData::StringArray(vec!["10m".to_string(), "3".to_string()]),
            ),
            ("blkioWeight", AstarteData::Integer(500)),
            (
                "blkioWeightDevicePath",
                AstarteData::StringArray(vec!["/dev/sda".to_string()]),
            ),
            (
                "blkioWeightDeviceWeight",
                AstarteData::IntegerArray(vec![500]),
            ),
            (
                "blkioDeviceReadBpsPath",
                AstarteData::StringArray(vec!["/dev/sda".to_string()]),
            ),
            (
                "blkioDeviceReadBpsRate",
                AstarteData::LongIntegerArray(vec![1_048_576]),
            ),
            (
                "blkioDeviceWriteBpsPath",
                AstarteData::StringArray(vec!["/dev/sda".to_string()]),
            ),
            (
                "blkioDeviceWriteBpsRate",
                AstarteData::LongIntegerArray(vec![1_048_576]),
            ),
            (
                "blkioDeviceReadIopsPath",
                AstarteData::StringArray(vec!["/dev/sda".to_string()]),
            ),
            (
                "blkioDeviceReadIopsRate",
                AstarteData::LongIntegerArray(vec![1000]),
            ),
            (
                "blkioDeviceWriteIopsPath",
                AstarteData::StringArray(vec!["/dev/sda".to_string()]),
            ),
            (
                "blkioDeviceWriteIopsRate",
                AstarteData::LongIntegerArray(vec![1000]),
            ),
            (
                "securityopt",
                AstarteData::StringArray(vec!["no-new-privileges".to_string()]),
            ),
            ("pidMode", AstarteData::String("host".to_string())),
            (
                "maskedPaths",
                AstarteData::StringArray(vec!["/proc/kcore".to_string()]),
            ),
            (
                "readOnlyPaths",
                AstarteData::StringArray(vec!["/proc/sys".to_string()]),
            ),
        ]
        .map(|(k, v)| (k.to_string(), v));

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateContainerRequest".to_string(),
            path: "/container".to_string(),
            data: Value::Object {
                data: AstarteObject::from_iter(fields),
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn create_container_request() {
        let deployment_id = ReqUuid(Uuid::new_v4());
        let image = create_image_req(deployment_id.0);
        let network = create_network_req(deployment_id.0);
        let volume = &create_volume_req(deployment_id.0);
        let device_mapping = create_device_mapping_req(deployment_id.0);
        let device_request = create_device_request(deployment_id.0);

        let expect = create_container_req(
            deployment_id.0,
            &image,
            volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let event = create_container_request_event(&expect);

        let request = CreateContainer::from_event(event).unwrap();

        assert_eq!(request, expect);
    }

    #[test]
    fn should_parse_port_binding() {
        let cases = [
            // ip:[hostPort:]containerPort[/protocol]
            (
                "1.1.1.1:80:90/udp",
                ParsedBind::new("90/udp", 90, Some("udp"), Some("1.1.1.1"), Some(80)),
            ),
            (
                "1.1.1.1:90/udp",
                ParsedBind::new("90/udp", 90, Some("udp"), Some("1.1.1.1"), None),
            ),
            (
                "1.1.1.1:90",
                ParsedBind::new("90", 90, None, Some("1.1.1.1"), None),
            ),
            // [hostPort:]containerPort[/protocol]
            (
                "80:90/udp",
                ParsedBind::new("90/udp", 90, Some("udp"), None, Some(80)),
            ),
            (
                "90/udp",
                ParsedBind::new("90/udp", 90, Some("udp"), None, None),
            ),
            ("90", ParsedBind::new("90", 90, None, None, None)),
        ];

        for (case, expected) in cases {
            let parsed = parse_port_binding(case).unwrap();

            assert_eq!(parsed, expected, "failed to parse {case}");
        }
    }
}
