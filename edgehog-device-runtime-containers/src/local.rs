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

//! Interface with the Containers locally

use std::collections::HashMap;

use bollard::models::ContainerStateStatusEnum;
use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptionsBuilder;
use bollard::secret::{ContainerInspectResponse, ContainerStatsResponse};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::containers::container::ContainerStatus;
use tracing::trace;
use uuid::Uuid;

use crate::container::ContainerId;
use crate::store::StateStore;
use crate::Docker;

#[cfg(feature = "__mock")]
use crate::client::DockerTrait;

/// Container handle for local clients
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    pub(crate) client: Docker,
    pub(crate) store: StateStore,
}

#[cfg_attr(feature = "__mock", mockall::automock)]
impl ContainerHandle {
    /// Create a new container handle
    pub fn new(client: Docker, store: StateStore) -> Self {
        Self { client, store }
    }

    async fn list_containers(
        &self,
        ids: &[(SqlUuid, Option<String>)],
        containers_status: Vec<ContainerStateStatusEnum>,
    ) -> eyre::Result<Vec<ContainerSummary>> {
        let local_ids = ids.iter().filter_map(|(_, local)| local.clone()).collect();
        let containers_status = containers_status
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

        let filters = HashMap::from_iter([
            ("id".to_string(), local_ids),
            ("status".to_string(), containers_status),
        ]);

        let opt = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();

        let containers = self.client.list_containers(Some(opt)).await?;

        Ok(containers)
    }

    /// List the container ids.
    ///
    /// If the container status list is empty, all the ids are returned. This includes the ones of
    /// container that have not been created yet. Which will have a container_id unset.
    pub async fn list_ids(
        &self,
        containers_status: Vec<ContainerStateStatusEnum>,
    ) -> eyre::Result<Vec<(Uuid, Option<String>)>> {
        let mut ids = self
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?;

        if containers_status.is_empty() {
            return Ok(ids
                .into_iter()
                .map(|(id, local_id)| (*id, local_id))
                .collect());
        }

        let containers = self.list_containers(&ids, containers_status).await?;

        // Used for binary searching the ids
        ids.sort_by(|(_, a), (_, b)| a.cmp(b));

        let ids = containers
            .into_iter()
            .filter_map(|summary| {
                let idx = summary
                    .id
                    .as_ref()
                    .and_then(|b| ids.binary_search_by(|(_, a)| a.as_ref().cmp(&Some(b))).ok())?;

                let id = *ids[idx].0;

                Some((id, summary.id))
            })
            .collect();

        Ok(ids)
    }

    /// List the container
    pub async fn list(
        &self,
        containers_status: Vec<ContainerStateStatusEnum>,
    ) -> eyre::Result<HashMap<Uuid, ContainerSummary>> {
        let mut ids = self
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?;

        let containers = self.list_containers(&ids, containers_status).await?;

        // Used for binary searching the ids
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

    /// Get all the containers
    ///
    /// This will list the containers and get all the information
    pub async fn get_all(
        &self,
        containers_status: Vec<ContainerStateStatusEnum>,
    ) -> eyre::Result<Vec<(Uuid, ContainerInspectResponse)>> {
        let ids = self
            .store
            .load_containers_in_state(vec![ContainerStatus::Stopped, ContainerStatus::Running])
            .await?;

        let mut containers = Vec::with_capacity(ids.len());
        for (id, local_id) in ids {
            let container = ContainerId::new(local_id, *id)
                .inspect(&self.client)
                .await?;

            let Some(container) = container else {
                trace!(%id, "skipped container is missing");

                continue;
            };

            let in_state = container
                .state
                .as_ref()
                .and_then(|state| state.status)
                .is_some_and(|status| {
                    containers_status.is_empty() || containers_status.contains(&status)
                });

            if !in_state {
                trace!(%id, "skipped container for state filter");

                continue;
            }

            containers.push((*id, container));
        }

        Ok(containers)
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
