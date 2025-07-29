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

use std::time::Duration;

use async_trait::async_trait;
use edgehog_containers::bollard::models::{ContainerStateStatusEnum, ContainerSummaryStateEnum};
use edgehog_containers::bollard::secret::{
    ContainerBlkioStatEntry, ContainerBlkioStats, ContainerCpuStats, ContainerCpuUsage,
    ContainerMemoryStats, ContainerNetworkStats, ContainerThrottlingData,
};
use edgehog_containers::local::ContainerHandle;
use edgehog_proto::containers::v1::containers_service_server::ContainersService;
use edgehog_proto::containers::v1::{
    BlkioStats, BlkioValue, Container, ContainerStatus, CpuStats, CpuThrottlingData, CpuUsage,
    GetRequest, GetResponse, ListRequest, ListResponse, ListStatusFilter, MemoryStats,
    NetworkStats, PidsStats, StartRequest, StatsRequest, StatsResponse, StopRequest,
};
use edgehog_proto::prost_types::Timestamp;
use edgehog_proto::tonic::{self, Response, Status};
use edgehog_store::models::containers::container::ContainerStatus as StoreContainerStatus;
use tokio::sync::mpsc;
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
        let status = inner.status_filter();

        let status = match status {
            ListStatusFilter::Unspecified => Vec::new(),
            ListStatusFilter::Running => vec![StoreContainerStatus::Running],
            ListStatusFilter::Stopped => vec![StoreContainerStatus::Stopped],
        };

        let containers = self
            .containers
            .list(status)
            .await
            .map_err(|err| {
                error!(error = format!("{err:#}"), "couldn't list containers");

                Status::internal("couldn't list containers")
            })?
            .into_iter()
            .map(|summary| Container {
                id: summary.id.unwrap_or_default(),
                created: summary
                    .created
                    .map(|seconds| Timestamp { seconds, nanos: 0 }),
                hostname: String::new(),
                image: summary.image_id.unwrap_or_default(),
                status: match summary.state {
                    None | Some(ContainerSummaryStateEnum::EMPTY) => ContainerStatus::Unspecified,
                    Some(ContainerSummaryStateEnum::CREATED) => ContainerStatus::Created,
                    Some(ContainerSummaryStateEnum::RUNNING) => ContainerStatus::Running,
                    Some(ContainerSummaryStateEnum::PAUSED) => ContainerStatus::Paused,
                    Some(ContainerSummaryStateEnum::RESTARTING) => ContainerStatus::Restarting,
                    Some(ContainerSummaryStateEnum::EXITED) => ContainerStatus::Exited,
                    Some(ContainerSummaryStateEnum::REMOVING) => ContainerStatus::Removing,
                    Some(ContainerSummaryStateEnum::DEAD) => ContainerStatus::Dead,
                }
                .into(),
                restart_count: 0,
            })
            .collect();

        Ok(ListResponse { containers }.into())
    }

    async fn get(
        &self,
        request: tonic::Request<GetRequest>,
    ) -> std::result::Result<tonic::Response<GetResponse>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let container = self
            .containers
            .get(id)
            .await
            .map_err(|err| {
                error!(error = format!("{err:#}"), "couldn't get container");

                Status::internal("couldn't get container")
            })?
            .map(|container| Container {
                id: id.to_string(),
                created: container.created.map(|created| Timestamp {
                    seconds: created.timestamp(),
                    nanos: 0,
                }),
                hostname: String::new(),
                image: container.image.unwrap_or_default(),
                status: match container.state.and_then(|state| state.status) {
                    None | Some(ContainerStateStatusEnum::EMPTY) => ContainerStatus::Unspecified,
                    Some(ContainerStateStatusEnum::CREATED) => ContainerStatus::Created,
                    Some(ContainerStateStatusEnum::RUNNING) => ContainerStatus::Running,
                    Some(ContainerStateStatusEnum::PAUSED) => ContainerStatus::Paused,
                    Some(ContainerStateStatusEnum::RESTARTING) => ContainerStatus::Paused,
                    Some(ContainerStateStatusEnum::REMOVING) => ContainerStatus::Removing,
                    Some(ContainerStateStatusEnum::EXITED) => ContainerStatus::Exited,
                    Some(ContainerStateStatusEnum::DEAD) => ContainerStatus::Dead,
                }
                .into(),
                restart_count: container
                    .restart_count
                    .and_then(|restarts| u32::try_from(restarts).ok())
                    .unwrap_or_default(),
            });

        Ok(GetResponse { container }.into())
    }

    async fn start(
        &self,
        request: tonic::Request<StartRequest>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let started = self.containers.start(id).await.map_err(|err| {
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

        let started = self.containers.stop(id).await.map_err(|err| {
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
        let inner = request.into_inner();
        let ids: Vec<Uuid> = inner
            .id
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<_, Status>>()?;

        let interval = inner
            .interval_seconds
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
            while !sender.tx.is_closed() {
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

struct StatsSender {
    tx: mpsc::Sender<Result<StatsResponse, Status>>,
    ids: Vec<Uuid>,
    containers: ContainerHandle,
}

impl StatsSender {
    async fn send(&self) -> eyre::Result<()> {
        for id in &self.ids {
            let res = self.containers.stats(id).await?;

            let stats = match res {
                Some(stats) => stats,
                None => {
                    debug!(%id, "missing stats");

                    continue;
                }
            };

            let pids_stas = stats.pids_stats.map(|pid_stats| PidsStats {
                current: pid_stats.current,
                limit: pid_stats.limit,
            });

            let value = StatsResponse {
                container_id: id.to_string(),
                read: stats.read.map(|read| Timestamp {
                    seconds: read.timestamp(),
                    nanos: read.timestamp_subsec_nanos().try_into().unwrap_or_default(),
                }),
                prepared: stats.preread.map(|preread| Timestamp {
                    seconds: preread.timestamp(),
                    nanos: preread
                        .timestamp_subsec_nanos()
                        .try_into()
                        .unwrap_or_default(),
                }),
                pids_stas,
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
            };

            self.tx
                .send_timeout(Ok(value), Duration::from_secs(10))
                .await?;
        }

        Ok(())
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
        io_service_bytes_recursize: convert_blkio_value(io_service_bytes_recursive),
        io_service_recursize: convert_blkio_value(io_serviced_recursive),
        io_queue_recursize: convert_blkio_value(io_queue_recursive),
        io_service_time_recursize: convert_blkio_value(io_service_time_recursive),
        io_wait_time_recursize: convert_blkio_value(io_wait_time_recursive),
        io_merged_recursize: convert_blkio_value(io_merged_recursive),
        io_time_recursize: convert_blkio_value(io_time_recursive),
        sectors_recursize: convert_blkio_value(sectors_recursive),
    }
}

fn convert_blkio_value(value: Option<Vec<ContainerBlkioStatEntry>>) -> Vec<BlkioValue> {
    value
        .unwrap_or_default()
        .into_iter()
        .map(|value| BlkioValue {
            major: value.major.unwrap_or_default(),
            minor: value.major.unwrap_or_default(),
            op: value.op.unwrap_or_default(),
            value: value.value.unwrap_or_default(),
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
            total_usage: total_usage.unwrap_or_default(),
            percpu_usage: percpu_usage.unwrap_or_default(),
            usage_in_kernelmode: usage_in_kernelmode.unwrap_or_default(),
            usage_in_usermode: usage_in_usermode.unwrap_or_default(),
        }
    });

    let throttling_data = throttling_data.map(|throttling| {
        let ContainerThrottlingData {
            periods,
            throttled_periods,
            throttled_time,
        } = throttling;

        CpuThrottlingData {
            periods: periods.unwrap_or_default(),
            throttled_periods: throttled_periods.unwrap_or_default(),
            throttled_time: throttled_time.unwrap_or_default(),
        }
    });

    CpuStats {
        cpu_usage,
        system_cpu_usage,
        online_cpus: online_cpus.map(u64::from),
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
        rx_bytes: rx_bytes.unwrap_or_default(),
        rx_packets: rx_packets.unwrap_or_default(),
        rx_errors: rx_errors.unwrap_or_default(),
        rx_dropped: rx_dropped.unwrap_or_default(),
        tx_bytes: tx_bytes.unwrap_or_default(),
        tx_packets: tx_packets.unwrap_or_default(),
        tx_errors: tx_errors.unwrap_or_default(),
        tx_dropped: tx_dropped.unwrap_or_default(),
    }
}
