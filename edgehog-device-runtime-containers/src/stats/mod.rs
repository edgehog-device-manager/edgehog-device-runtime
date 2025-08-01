// This file is part of Edgehog .
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

//! Gather statistics of the container and sends them to Astarte.

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::Client;
use edgehog_store::models::containers::container::ContainerStatus;
use tracing::{debug, error, instrument, trace};
use uuid::Uuid;

use crate::container::ContainerId;
use crate::store::StateStore;
use crate::Docker;

use self::network::ContainerNetworkStats;

pub(crate) mod network;

/// Handles the events received from the container runtime
#[derive(Debug)]
pub struct StatsMonitor<D> {
    client: Docker,
    device: D,
    store: StateStore,
}

impl<D> StatsMonitor<D>
where
    D: Client + Send + Sync + 'static,
{
    /// Creates a new instance.
    pub fn new(client: Docker, device: D, store: StateStore) -> Self {
        Self {
            client,
            device,
            store,
        }
    }

    /// Gathers and sends the statistics to Astarte.
    #[instrument(skip(self))]
    pub async fn gather(&mut self) -> eyre::Result<()> {
        let containers: Vec<ContainerId> = self
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?
            .into_iter()
            .map(|(id, local_id)| ContainerId::new(local_id, *id))
            .collect();

        trace!(len = containers.len(), "loaded containers from store");

        for container in containers {
            let stats = match container.stats(&self.client).await {
                Ok(Some(stats)) => stats,
                Ok(None) => continue,
                Err(err) => {
                    error!(%container, error = %format!("{:#}", eyre::Report::new(err)), "couldn't get container stasts");

                    continue;
                }
            };

            let timestamp = stats.read.unwrap_or_else(|| {
                debug!("missing read timestmp, genereting one");

                Utc::now()
            });

            match stats.networks {
                Some(networks) => {
                    let networks = ContainerNetworkStats::from_stats(networks);

                    for net in networks {
                        net.send(&container.name, &mut self.device, &timestamp)
                            .await;
                    }
                }
                None => {
                    debug!("missing network stats");
                }
            }
        }

        Ok(())
    }
}

/// Send a some stats.
///
/// The interface need to be explicit timestamp.
trait Metric: TryInto<AstarteObject> {
    const INTERFACE: &'static str;
    // Like "container network"
    const METRIC_NAME: &'static str;

    async fn send<D>(self, id: &Uuid, device: &mut D, timestamp: &DateTime<Utc>)
    where
        D: Client + Sync + 'static,
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
