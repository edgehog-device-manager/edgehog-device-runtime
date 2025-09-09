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

//! Interface with the Containers locally

use std::collections::HashMap;

use bollard::models::ContainerStateStatusEnum;
use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptionsBuilder;
use bollard::secret::{ContainerInspectResponse, ContainerStatsResponse};
use edgehog_store::models::containers::container::ContainerStatus;
use uuid::Uuid;

use crate::container::ContainerId;
use crate::store::StateStore;
use crate::Docker;

#[cfg(feature = "__mock")]
use crate::client::DockerTrait;

/// Container handle for local clients
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    client: Docker,
    store: StateStore,
}

impl ContainerHandle {
    /// Create a new container handle
    pub fn new(client: Docker, store: StateStore) -> Self {
        Self { client, store }
    }

    /// List the container
    pub async fn list(
        &self,
        container_status: Vec<ContainerStateStatusEnum>,
    ) -> eyre::Result<HashMap<Uuid, ContainerSummary>> {
        // filter empty
        let container_status = container_status
            .into_iter()
            .filter_map(|s| match s {
                ContainerStateStatusEnum::EMPTY => None,
                ContainerStateStatusEnum::CREATED
                | ContainerStateStatusEnum::RUNNING
                | ContainerStateStatusEnum::PAUSED
                | ContainerStateStatusEnum::RESTARTING
                | ContainerStateStatusEnum::REMOVING
                | ContainerStateStatusEnum::EXITED
                | ContainerStateStatusEnum::DEAD => Some(s.to_string()),
            })
            .collect();

        let mut ids = self
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?;

        let local_ids = ids.iter().filter_map(|(_, local)| local.clone()).collect();

        let filters = HashMap::from_iter([
            ("id".to_string(), local_ids),
            ("status".to_string(), container_status),
        ]);

        let opt = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();

        let containers = self.client.list_containers(Some(opt)).await?;

        ids.sort_by(|(_, a), (_, b)| a.cmp(b));

        let map = containers
            .into_iter()
            .filter_map(|summary| {
                let idx = summary
                    .id
                    .as_ref()
                    .and_then(|b| ids.binary_search_by(|(_, a)| a.as_ref().cmp(&Some(b))).ok())?;

                let id = *ids[idx].0;

                Some((id, summary))
            })
            .collect();

        Ok(map)
    }

    /// Get the container
    pub async fn get(&self, id: Uuid) -> eyre::Result<Option<ContainerInspectResponse>> {
        let local_id = self.store.load_container_local_id(id).await?;

        let container = ContainerId::new(local_id, id).inspect(&self.client).await?;

        Ok(container)
    }

    /// Start the container
    pub async fn start(&self, id: Uuid) -> eyre::Result<Option<()>> {
        let local_id = self.store.load_container_local_id(id).await?;

        let started = ContainerId::new(local_id, id).start(&self.client).await?;

        Ok(started)
    }

    /// Stop the container
    pub async fn stop(&self, id: Uuid) -> eyre::Result<Option<()>> {
        let local_id = self.store.load_container_local_id(id).await?;

        let container = ContainerId::new(local_id, id).stop(&self.client).await?;

        Ok(container)
    }

    /// Stats for a container
    pub async fn stats(&self, id: &Uuid) -> eyre::Result<Option<ContainerStatsResponse>> {
        let local_id = self.store.load_container_local_id(*id).await?;

        let container = ContainerId::new(local_id, *id).stats(&self.client).await?;

        Ok(container)
    }

    /// Stats for all the containers
    pub async fn all_stats(&self) -> eyre::Result<Vec<(Uuid, ContainerStatsResponse)>> {
        let ids = self
            .store
            // Containers that have been created
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?;

        let mut stats = Vec::with_capacity(ids.len());

        for (id, local_id) in ids {
            if let Some(stat) = ContainerId::new(local_id, *id).stats(&self.client).await? {
                stats.push((*id, stat));
            }
        }

        Ok(stats)
    }
}
