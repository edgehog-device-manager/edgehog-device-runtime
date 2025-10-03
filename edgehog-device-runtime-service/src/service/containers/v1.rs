// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use edgehog_containers::bollard;
use edgehog_containers::bollard::models::{
    ContainerBlkioStatEntry, ContainerBlkioStats, ContainerCpuStats, ContainerCpuUsage,
    ContainerMemoryStats, ContainerNetworkStats, ContainerStateStatusEnum,
    ContainerSummaryStateEnum, ContainerThrottlingData, PortBinding, PortTypeEnum,
};
use edgehog_containers::bollard::secret::{ContainerInspectResponse, ContainerStatsResponse};
use edgehog_containers::local::ContainerHandle;
use edgehog_proto::containers::v1::containers_service_server::ContainersService;
use edgehog_proto::containers::v1::list_response::Info;
use edgehog_proto::containers::v1::{
    BlkioStats, BlkioValue, Container, ContainerId, ContainerState, ContainerSummary, CpuStats,
    CpuThrottlingData, CpuUsage, GetRequest, GetResponse, ListInfo, ListInfoFull, ListInfoIds,
    ListInfoSummary, ListRequest, ListResponse, MemoryStats, NetworkStats, PidsStats, PortMapping,
    PortType, StartRequest, StatsRequest, StatsResponse, StopRequest,
};
use edgehog_proto::prost_types::Timestamp;
use edgehog_proto::tonic::{self, Response, Status};
use eyre::{bail, Context};
use tokio::sync::{mpsc, OnceCell};
use tokio_stream::wrappers::ReceiverStream;
use tracing::{debug, error};
use uuid::Uuid;

use crate::service::EdgehogService;

#[async_trait]
impl ContainersService for EdgehogService {
    type StatsStream = ReceiverStream<std::result::Result<StatsResponse, tonic::Status>>;

    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> std::result::Result<tonic::Response<ListResponse>, tonic::Status> {
        let inner = request.into_inner();
        let status = inner
            .container_state_filter()
            .map(|state| match state {
                ContainerState::Unspecified => ContainerStateStatusEnum::EMPTY,
                ContainerState::Created => ContainerStateStatusEnum::CREATED,
                ContainerState::Running => ContainerStateStatusEnum::RUNNING,
                ContainerState::Paused => ContainerStateStatusEnum::PAUSED,
                ContainerState::Restarting => ContainerStateStatusEnum::RESTARTING,
                ContainerState::Removing => ContainerStateStatusEnum::REMOVING,
                ContainerState::Exited => ContainerStateStatusEnum::EXITED,
                ContainerState::Dead => ContainerStateStatusEnum::DEAD,
            })
            .collect();

        let info = match inner.list_info() {
            ListInfo::Unspecified | ListInfo::Summary => {
                let containers = self
                    .container_handle()?
                    .list(status)
                    .await
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?
                    .into_iter()
                    .map(|(id, summary)| convert_container_summary(id, summary))
                    .collect();

                Info::Summary(ListInfoSummary { containers })
            }
            ListInfo::Ids => {
                let ids = self
                    .container_handle()?
                    .list_ids(status)
                    .await
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?
                    .into_iter()
                    .map(|(id, container_id)| ContainerId {
                        id: id.to_string(),
                        container_id,
                    })
                    .collect();

                Info::Ids(ListInfoIds { ids })
            }
            ListInfo::Full => {
                let containers = self
                    .container_handle()?
                    .get_all(status)
                    .await
                    .and_then(|containers| {
                        containers
                            .into_iter()
                            .map(|(id, container)| convert_container(&id, container))
                            .collect::<eyre::Result<Vec<Container>>>()
                    })
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?;

                Info::Full(ListInfoFull { containers })
            }
        };

        Ok(ListResponse { info: Some(info) }.into())
    }

    async fn get(
        &self,
        request: tonic::Request<GetRequest>,
    ) -> std::result::Result<tonic::Response<GetResponse>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let container = self
            .container_handle()?
            .get(id)
            .await
            .and_then(|container| {
                let Some(container) = container else {
                    return Ok(None);
                };

                convert_container(&id, container).map(Some)
            })
            .map_err(|err| {
                error!(error = format!("{err:#}"), "couldn't get container");

                Status::internal("couldn't get container")
            })?;

        Ok(GetResponse { container }.into())
    }

    async fn start(
        &self,
        request: tonic::Request<StartRequest>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let started = self.container_handle()?.start(id).await.map_err(|err| {
            error!(error = format!("{err:#}"), "couldn't start the container");

            Status::internal("couldn't start the container")
        })?;

        if started.is_none() {
            return Err(Status::not_found("container doesn't exist"));
        }

        Ok(Response::new(()))
    }

    async fn stop(
        &self,
        request: tonic::Request<StopRequest>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let started = self.container_handle()?.stop(id).await.map_err(|err| {
            error!(error = format!("{err:#}"), "couldn't stop the container");

            Status::internal("couldn't stop the container")
        })?;

        if started.is_none() {
            return Err(Status::not_found("container doesn't exist"));
        }

        Ok(Response::new(()))
    }

    /// Stream container statistics.
    async fn stats(
        &self,
        request: tonic::Request<StatsRequest>,
    ) -> std::result::Result<tonic::Response<Self::StatsStream>, tonic::Status> {
        let StatsRequest {
            ids,
            interval_seconds,
            limit,
        } = request.into_inner();
        let ids = ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<Uuid>, Status>>()?;
        let interval = interval_seconds
            .map(|interval| {
                u64::try_from(interval).map_err(|err| {
                    error!(
                        interval,
                        error = format!("{err:#}"),
                        "invalid interval for statistics"
                    );

                    Status::invalid_argument("invalid interval")
                })
            })
            .transpose()?
            .unwrap_or(10);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut interval = tokio::time::interval(Duration::from_secs(interval));
        let containers = self.containers.clone();

        let sender = StatsSender {
            tx,
            ids,
            containers,
        };

        tokio::spawn(async move {
            let limit = limit.unwrap_or(u64::MAX);
            let range = (0..limit).take_while(|_| !sender.tx.is_closed());

            for _ in range {
                if let Err(err) = sender.send().await {
                    error!(error = format!("{err:#}"), "couldn't send stats");

                    break;
                }

                interval.tick().await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

fn parse_id(id: &str) -> std::result::Result<Uuid, Status> {
    Uuid::parse_str(id).map_err(|err| {
        error!(id, error = format!("{err:#}"), "invalid id");

        Status::invalid_argument("invalid id")
    })
}

fn parse_port_binding(
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

struct StatsSender {
    tx: mpsc::Sender<Result<StatsResponse, Status>>,
    ids: Vec<Uuid>,
    containers: Arc<OnceCell<ContainerHandle>>,
}

impl StatsSender {
    fn container_handle(&self) -> Result<&ContainerHandle, Status> {
        self.containers.get().ok_or_else(|| {
            error!("container service is not available");

            Status::unavailable("container service not available")
        })
    }
    async fn send_all(&self) -> eyre::Result<()> {
        let stats = self.container_handle()?.all_stats().await?;

        for (id, stat) in stats {
            let value = convert_stats(&id, stat);

            self.tx
                .send_timeout(Ok(value), Duration::from_secs(10))
                .await?;
        }

        Ok(())
    }

    async fn send(&self) -> eyre::Result<()> {
        if self.ids.is_empty() {
            self.send_all().await?;

            return Ok(());
        }

        for id in &self.ids {
            let res = self.container_handle()?.stats(id).await?;

            let stats = match res {
                Some(stats) => stats,
                None => {
                    debug!(%id, "missing stats");

                    continue;
                }
            };

            let value = convert_stats(id, stats);

            self.tx
                .send_timeout(Ok(value), Duration::from_secs(10))
                .await?;
        }

        Ok(())
    }
}

fn convert_stats(id: &Uuid, stats: ContainerStatsResponse) -> StatsResponse {
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

fn convert_container_summary(
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

fn convert_container(id: &Uuid, container: ContainerInspectResponse) -> eyre::Result<Container> {
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
