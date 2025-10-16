// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::collections::HashMap;
use std::result::Result;

use edgehog_containers::bollard;
use edgehog_containers::bollard::models::{
    ContainerBlkioStatEntry, ContainerBlkioStats, ContainerCpuStats, ContainerCpuUsage,
    ContainerMemoryStats, ContainerNetworkStats, ContainerStateStatusEnum,
    ContainerSummaryStateEnum, ContainerThrottlingData, PortBinding, PortTypeEnum,
};
use edgehog_containers::bollard::secret::{ContainerInspectResponse, ContainerStatsResponse};
use edgehog_proto::containers::v1::{
    BlkioStats, BlkioValue, Container, ContainerId, ContainerState, ContainerSummary, CpuStats,
    CpuThrottlingData, CpuUsage, MemoryStats, NetworkStats, PidsStats, PortMapping, PortType,
    StatsResponse,
};
use edgehog_proto::prost_types::Timestamp;
use edgehog_proto::tonic::Status;
use eyre::{bail, Context};
use tracing::error;
use uuid::Uuid;

pub(crate) fn parse_id(id: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(id).map_err(|err| {
        error!(id, error = format!("{err:#}"), "invalid id");

        Status::invalid_argument("invalid id")
    })
}

pub(crate) fn parse_port_binding(
    port_bindings: HashMap<String, Option<Vec<PortBinding>>>,
) -> eyre::Result<Vec<PortMapping>> {
    port_bindings
        .into_iter()
        .try_fold(Vec::new(), |mut acc, (key, binding)| {
            let (port, proto) = parse_port_binding_key(&key)?;

            // push the single port
            let Some(binding) = binding else {
                acc.push(PortMapping {
                    ip: None,
                    private_port: port.into(),
                    public_port: None,
                    protocol: proto.into(),
                });

                return Ok(acc);
            };

            acc.reserve(binding.len());

            for bind in binding {
                let public_port = bind
                    .host_port
                    .map(|port| {
                        port.parse::<u16>()
                            .map(u32::from)
                            .wrap_err("invalid host port")
                    })
                    .transpose()?;

                acc.push(PortMapping {
                    ip: bind.host_ip,
                    private_port: port.into(),
                    public_port,
                    protocol: proto.into(),
                });
            }

            Ok(acc)
        })
}

fn parse_port_binding_key(key: &str) -> eyre::Result<(u16, PortType)> {
    let (port, proto) = key.split_once('/').unwrap_or((key, ""));

    let port = port.parse::<u16>()?;

    let proto = match proto {
        "" => PortType::Unspecified,
        "tcp" => PortType::Tcp,
        "udp" => PortType::Udp,
        "scpt" => PortType::Sctp,
        _ => bail!("invalid protocol: {proto}"),
    };

    Ok((port, proto))
}

pub(crate) fn convert_stats(id: &Uuid, stats: ContainerStatsResponse) -> StatsResponse {
    let pids_stats = stats.pids_stats.map(|pid_stats| PidsStats {
        current: pid_stats.current,
        limit: pid_stats.limit,
    });

    StatsResponse {
        id: Some(ContainerId {
            id: id.to_string(),
            container_id: stats.id,
        }),
        read: stats.read.map(|read| Timestamp {
            seconds: read.timestamp(),
            nanos: read.timestamp_subsec_nanos().try_into().unwrap_or_default(),
        }),
        preread: stats.preread.map(|preread| Timestamp {
            seconds: preread.timestamp(),
            nanos: preread
                .timestamp_subsec_nanos()
                .try_into()
                .unwrap_or_default(),
        }),
        pids_stats,
        blkio_stats: stats.blkio_stats.map(convert_blkio_stats),
        cpu_stats: stats.cpu_stats.map(convert_cpu_stats),
        precpu_stats: stats.precpu_stats.map(convert_cpu_stats),
        memory_stats: stats.memory_stats.map(convert_memory_stats),
        networks: stats
            .networks
            .unwrap_or_default()
            .into_iter()
            .map(|(interface, value)| (interface, convert_network_stats(value)))
            .collect(),
    }
}

fn convert_blkio_stats(value: ContainerBlkioStats) -> BlkioStats {
    let ContainerBlkioStats {
        io_service_bytes_recursive,
        io_serviced_recursive,
        io_queue_recursive,
        io_service_time_recursive,
        io_wait_time_recursive,
        io_merged_recursive,
        io_time_recursive,
        sectors_recursive,
    } = value;

    BlkioStats {
        io_service_bytes_recursive: convert_blkio_value(io_service_bytes_recursive),
        io_serviced_recursive: convert_blkio_value(io_serviced_recursive),
        io_queue_recursive: convert_blkio_value(io_queue_recursive),
        io_service_time_recursive: convert_blkio_value(io_service_time_recursive),
        io_wait_time_recursive: convert_blkio_value(io_wait_time_recursive),
        io_merged_recursive: convert_blkio_value(io_merged_recursive),
        io_time_recursive: convert_blkio_value(io_time_recursive),
        sectors_recursive: convert_blkio_value(sectors_recursive),
    }
}

fn convert_blkio_value(value: Option<Vec<ContainerBlkioStatEntry>>) -> Vec<BlkioValue> {
    value
        .unwrap_or_default()
        .into_iter()
        .map(|value| BlkioValue {
            major: value.major,
            minor: value.major,
            op: value.op,
            value: value.value,
        })
        .collect()
}

fn convert_cpu_stats(value: ContainerCpuStats) -> CpuStats {
    let ContainerCpuStats {
        cpu_usage,
        system_cpu_usage,
        online_cpus,
        throttling_data,
    } = value;

    let cpu_usage = cpu_usage.map(|usage| {
        let ContainerCpuUsage {
            total_usage,
            percpu_usage,
            usage_in_kernelmode,
            usage_in_usermode,
        } = usage;

        CpuUsage {
            total_usage,
            percpu_usage: percpu_usage.unwrap_or_default(),
            usage_in_kernelmode,
            usage_in_usermode,
        }
    });

    let throttling_data = throttling_data.map(|throttling| {
        let ContainerThrottlingData {
            periods,
            throttled_periods,
            throttled_time,
        } = throttling;

        CpuThrottlingData {
            periods,
            throttled_periods,
            throttled_time,
        }
    });

    CpuStats {
        cpu_usage,
        system_cpu_usage,
        online_cpus,
        throttling_data,
    }
}

fn convert_memory_stats(value: ContainerMemoryStats) -> MemoryStats {
    let ContainerMemoryStats {
        usage,
        max_usage,
        stats,
        failcnt,
        limit,
        // Windows specific
        commitbytes: _,
        commitpeakbytes: _,
        privateworkingset: _,
    } = value;

    MemoryStats {
        usage,
        max_usage,
        stats: stats.unwrap_or_default(),
        failcnt,
        limit,
    }
}

fn convert_network_stats(value: ContainerNetworkStats) -> NetworkStats {
    let ContainerNetworkStats {
        rx_bytes,
        rx_packets,
        rx_errors,
        rx_dropped,
        tx_bytes,
        tx_packets,
        tx_errors,
        tx_dropped,
        // Windows specific
        endpoint_id: _,
        instance_id: _,
    } = value;

    NetworkStats {
        rx_bytes,
        rx_packets,
        rx_errors,
        rx_dropped,
        tx_bytes,
        tx_packets,
        tx_errors,
        tx_dropped,
    }
}

pub(crate) fn convert_container_summary(
    id: Uuid,
    summary: bollard::models::ContainerSummary,
) -> ContainerSummary {
    ContainerSummary {
        id: Some(ContainerId {
            id: id.to_string(),
            container_id: summary.id,
        }),
        created: summary
            .created
            .map(|seconds| Timestamp { seconds, nanos: 0 }),
        image: summary.image_id.unwrap_or_default(),
        state: match summary.state {
            None | Some(ContainerSummaryStateEnum::EMPTY) => ContainerState::Unspecified,
            Some(ContainerSummaryStateEnum::CREATED) => ContainerState::Created,
            Some(ContainerSummaryStateEnum::RUNNING) => ContainerState::Running,
            Some(ContainerSummaryStateEnum::PAUSED) => ContainerState::Paused,
            Some(ContainerSummaryStateEnum::RESTARTING) => ContainerState::Restarting,
            Some(ContainerSummaryStateEnum::EXITED) => ContainerState::Exited,
            Some(ContainerSummaryStateEnum::REMOVING) => ContainerState::Removing,
            Some(ContainerSummaryStateEnum::DEAD) => ContainerState::Dead,
        }
        .into(),
        ports: summary
            .ports
            .unwrap_or_default()
            .into_iter()
            .map(|port| PortMapping {
                ip: port.ip,
                private_port: port.private_port.into(),
                public_port: port.public_port.map(u32::from),
                protocol: port
                    .typ
                    .map(|typ| match typ {
                        PortTypeEnum::EMPTY => PortType::Unspecified,
                        PortTypeEnum::TCP => PortType::Tcp,
                        PortTypeEnum::UDP => PortType::Udp,
                        PortTypeEnum::SCTP => PortType::Sctp,
                    })
                    .unwrap_or(PortType::Unspecified)
                    .into(),
            })
            .collect(),
    }
}

pub(crate) fn convert_container(
    id: &Uuid,
    container: ContainerInspectResponse,
) -> eyre::Result<Container> {
    let ports = parse_port_binding(
        container
            .host_config
            .unwrap_or_default()
            .port_bindings
            .unwrap_or_default(),
    )?;

    Ok(Container {
        id: Some(ContainerId {
            id: id.to_string(),
            container_id: container.id,
        }),
        created: container.created.map(|created| Timestamp {
            seconds: created.timestamp(),
            nanos: 0,
        }),
        image: container.image.unwrap_or_default(),
        state: match container.state.and_then(|state| state.status) {
            None | Some(ContainerStateStatusEnum::EMPTY) => ContainerState::Unspecified,
            Some(ContainerStateStatusEnum::CREATED) => ContainerState::Created,
            Some(ContainerStateStatusEnum::RUNNING) => ContainerState::Running,
            Some(ContainerStateStatusEnum::PAUSED) => ContainerState::Paused,
            Some(ContainerStateStatusEnum::RESTARTING) => ContainerState::Paused,
            Some(ContainerStateStatusEnum::REMOVING) => ContainerState::Removing,
            Some(ContainerStateStatusEnum::EXITED) => ContainerState::Exited,
            Some(ContainerStateStatusEnum::DEAD) => ContainerState::Dead,
        }
        .into(),
        restart_count: container
            .restart_count
            .and_then(|restarts| u32::try_from(restarts).ok())
            .unwrap_or_default(),
        ports,
    })
}
