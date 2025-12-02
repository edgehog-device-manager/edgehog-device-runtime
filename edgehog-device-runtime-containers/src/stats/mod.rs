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

//! Gather statistics of the container and sends them to Astarte.

use std::sync::Arc;

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::Client;
use bollard::secret::ContainerStatsResponse;
use edgehog_store::models::containers::container::ContainerStatus;
use edgehog_store::models::containers::volume::VolumeStatus;
use tokio::sync::OnceCell;
use tracing::{debug, error, instrument, trace};
use uuid::Uuid;

use crate::container::ContainerId;
use crate::local::ContainerHandle;
use crate::store::StateStore;
use crate::volume::VolumeId;
use crate::Docker;

use self::blkio::ContainerBlkio;
use self::cpu::ContainerCpu;
use self::memory::{ContainerMemory, ContainerMemoryStats};
use self::network::ContainerNetworkStats;
use self::procs::ContainerProcesses;
use self::volume::VolumeUsage;

mod blkio;
mod cpu;
mod memory;
mod network;
mod procs;
mod volume;

/// Handles the events received from the container runtime
#[derive(Debug)]
pub struct StatsMonitor {
    handle: Arc<OnceCell<ContainerHandle>>,
}

impl StatsMonitor {
    /// Creates a new instance.
    pub fn new(handle: Arc<OnceCell<ContainerHandle>>) -> Self {
        Self { handle }
    }

    /// Creates an initialized instance.
    pub fn with_handle(client: Docker, store: StateStore) -> Self {
        Self {
            handle: Arc::new(OnceCell::const_new_with(ContainerHandle::new(
                client, store,
            ))),
        }
    }

    fn get_handle(&self) -> Option<&ContainerHandle> {
        let handle = self.handle.get();

        if handle.is_none() {
            debug!("handle not yet initialized");
        }

        handle
    }

    /// Loads the container ids from the storage
    async fn load_container_ids(&self) -> Option<Vec<ContainerId>> {
        let handle = self.get_handle()?;

        let containers: Vec<ContainerId> = handle
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await
            .inspect_err(|err| error!(error = format!("{err:#}"), "couldn't load containers"))
            .ok()?
            .into_iter()
            .map(|(id, local_id)| ContainerId::new(local_id, *id))
            .collect();

        trace!(len = containers.len(), "loaded containers from store");

        Some(containers)
    }

    /// Reads the stats of a container
    #[instrument(skip(self))]
    async fn read_stats(
        &self,
        container: &ContainerId,
    ) -> Option<(ContainerStatsResponse, DateTime<Utc>)> {
        let handle = self.handle.get()?;

        let stats = match container.stats(&handle.client).await {
            Ok(Some(stats)) => stats,
            Ok(None) => {
                debug!("missing stats for container");

                return None;
            }
            Err(err) => {
                error!(%container, error = %format!("{:#}", eyre::Report::new(err)), "couldn't get container stasts");

                return None;
            }
        };

        let timestamp = stats.read.unwrap_or_else(|| {
            debug!("missing read timestamp, generating one");

            Utc::now()
        });

        Some((stats, timestamp))
    }

    /// Sends the container network stats
    #[instrument(skip(self, device))]
    pub async fn network<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                continue;
            };

            if let Some(networks) = stats.networks {
                let networks = ContainerNetworkStats::from_stats(networks);

                for net in networks {
                    net.send(&container.name, device, &timestamp).await;
                }
            } else {
                debug!("missing network stats");
            }
        }
    }

    /// Sends the container memory stats
    #[instrument(skip(self, device))]
    pub async fn memory<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                debug!("missing stats for container");

                continue;
            };

            if let Some(memory) = stats.memory_stats {
                ContainerMemory::from(&memory)
                    .send(&container.name, device, &timestamp)
                    .await;
            } else {
                debug!("missing memory stats");
            }
        }
    }

    /// Sends the container memory stats for cgroup v2
    #[instrument(skip(self, device))]
    pub async fn memory_stats<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                debug!("missing stats for container");

                continue;
            };

            if let Some(memory) = stats.memory_stats {
                if let Some(memory_stats) = memory.stats {
                    let memory = ContainerMemoryStats::from_stats(memory_stats);

                    for mem in memory {
                        mem.send(&container.name, device, &timestamp).await;
                    }
                } else {
                    trace!("missing cgroups v2 memory stats");
                }
            } else {
                debug!("missing memory stats");
            }
        }
    }

    /// Sends the container cpu stats
    #[instrument(skip(self, device))]
    pub async fn cpu<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                debug!("missing stats for container");

                continue;
            };

            if let Some(cpu) = stats.cpu_stats {
                ContainerCpu::from_stats(cpu, stats.precpu_stats.unwrap_or_default())
                    .send(&container.name, device, &timestamp)
                    .await;
            } else {
                debug!("missing cpu stats");
            }
        }
    }

    /// Sends the container blkio stats
    #[instrument(skip(self, device))]
    pub async fn blkio<D>(&mut self, device: &mut D)
    where
        D: Client + Send,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                debug!("missing stats for container");

                continue;
            };

            if let Some(blkio) = stats.blkio_stats {
                let blkio = ContainerBlkio::from_stats(blkio);
                for value in blkio {
                    value.send(&container.name, device, &timestamp).await;
                }
            } else {
                debug!("missing blkio stats");
            }
        }
    }

    /// Sends the container pids stats
    #[instrument(skip(self, device))]
    pub async fn pids<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(containers) = self.load_container_ids().await else {
            return;
        };

        for container in containers {
            let Some((stats, timestamp)) = self.read_stats(&container).await else {
                debug!("missing stats for container");

                continue;
            };

            if let Some(pids) = stats.pids_stats {
                ContainerProcesses::from(pids)
                    .send(&container.name, device, &timestamp)
                    .await;
            } else {
                debug!("missing pids stats");
            }
        }
    }

    /// Sends the voulume usage stats
    #[instrument(skip(self, device))]
    pub async fn volumes<D>(&mut self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let Some(handle) = self.get_handle() else {
            return;
        };

        let volumes: Vec<VolumeId> = handle
            .store
            .load_volumes_in_state(VolumeStatus::Created)
            .await
            .inspect_err(|err| error!(error = format!("{err:#}"), "couldn't load volumes"))
            .unwrap_or_default()
            .into_iter()
            .map(|id| VolumeId::new(*id))
            .collect();

        trace!(len = volumes.len(), "loaded volumes from store");

        for volume in volumes {
            match volume.inspect(&handle.client).await {
                Ok(Some(info)) => {
                    VolumeUsage::from(info)
                        .send(&volume.name, device, &Utc::now())
                        .await;
                }
                Ok(None) => {}
                Err(err) => {
                    error!(%volume, error = %format!("{:#}", eyre::Report::new(err)), "couldn't get container stasts");

                    continue;
                }
            };
        }
    }
}

/// Send metrics to Astarte for an Interface.
///
/// It will handle any error raised by logging it. The interface need to be explicit timestamp.
trait Metric: TryInto<AstarteObject> {
    const INTERFACE: &'static str;
    // Like "container network"
    const METRIC_NAME: &'static str;

    async fn send<D>(self, id: &Uuid, device: &mut D, timestamp: &DateTime<Utc>)
    where
        D: Client + Send,
        Self::Error: std::error::Error + Send + Sync + 'static,
    {
        let data: AstarteObject = match self.try_into() {
            Ok(data) => data,
            Err(err) => {
                error!(container=%id, error = format!("{:#}", eyre::Report::new(err)), "couldn't convert {} stats", Self::METRIC_NAME);

                return;
            }
        };

        let res = device
            .send_object_with_timestamp(Self::INTERFACE, &format!("/{id}"), data, *timestamp)
            .await;

        if let Err(err) = res {
            error!(container=%id, error = format!("{:#}", eyre::Report::new(err)), "couldn't send {} stats", Self::METRIC_NAME);
        }
    }
}

trait IntoAstarteExt {
    type Out;

    fn into_astarte(self) -> Self::Out;
}

impl IntoAstarteExt for Option<u32> {
    type Out = i32;

    fn into_astarte(self) -> Self::Out {
        self.unwrap_or_default().try_into().unwrap_or(i32::MAX)
    }
}

impl IntoAstarteExt for Option<u64> {
    type Out = i64;

    fn into_astarte(self) -> Self::Out {
        self.unwrap_or_default().try_into().unwrap_or(i64::MAX)
    }
}

impl IntoAstarteExt for Option<Vec<u64>> {
    type Out = Vec<i64>;

    fn into_astarte(self) -> Self::Out {
        self.unwrap_or_default()
            .into_iter()
            .map(|value| value.try_into().unwrap_or(i64::MAX))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use edgehog_store::db;
    use tempfile::TempDir;

    use crate::store::StateStore;

    use super::*;

    #[tokio::test]
    async fn create_new() {
        let tmp = TempDir::with_prefix("fetch_by_local_id").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let client = crate::Docker::connect().await.unwrap();

        let _stats = StatsMonitor::with_handle(client, store);
    }

    #[test]
    fn check_into_astarte_ext() {
        let u32_val: Option<u32> = Some(42);
        assert_eq!(u32_val.into_astarte(), 42i32);

        let u32_none: Option<u32> = None;
        assert_eq!(u32_none.into_astarte(), 0i32);

        let u32_max: Option<u32> = Some(u32::MAX);
        assert_eq!(u32_max.into_astarte(), i32::MAX);

        let u64_val: Option<u64> = Some(12345);
        assert_eq!(u64_val.into_astarte(), 12345i64);

        let u64_none: Option<u64> = None;
        assert_eq!(u64_none.into_astarte(), 0i64);

        let u64_max: Option<u64> = Some(u64::MAX);
        assert_eq!(u64_max.into_astarte(), i64::MAX);

        let vec_val: Option<Vec<u64>> = Some(vec![10, 20, 30]);
        assert_eq!(vec_val.into_astarte(), vec![10i64, 20i64, 30i64]);

        let vec_none: Option<Vec<u64>> = None;
        assert_eq!(vec_none.into_astarte(), Vec::<i64>::new());

        let vec_empty: Option<Vec<u64>> = Some(vec![]);
        assert_eq!(vec_empty.into_astarte(), Vec::<i64>::new());

        let vec_mixed: Option<Vec<u64>> = Some(vec![100, u64::MAX, 200]);
        assert_eq!(vec_mixed.into_astarte(), vec![100i64, i64::MAX, 200i64]);
    }
}
