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

use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptionsBuilder;
use bollard::secret::{ContainerInspectResponse, ContainerStatsResponse};
use edgehog_store::models::containers::container::ContainerStatus;
use uuid::Uuid;

use crate::container::ContainerId;
use crate::store::StateStore;
use crate::{Docker, DockerTrait};

/// Container handle for local clients
#[derive(Debug, Clone)]
pub struct ContainerHandle {
    client: Docker,
    store: StateStore,
}

impl ContainerHandle {
    /// Create a new container handle
    pub fn new(clinet: Docker, store: StateStore) -> Self {
        Self {
            client: clinet,
            store,
        }
    }

    /// List the container
    pub async fn list(&self, status: Vec<ContainerStatus>) -> eyre::Result<Vec<ContainerSummary>> {
        let local_ids: Vec<String> = self
            .store
            .load_containers_in_state(status)
            .await?
            .into_iter()
            .filter_map(|(_id, local_id)| local_id)
            .collect();

        let filters = HashMap::from_iter([("id".to_string(), local_ids)]);

        let opt = ListContainersOptionsBuilder::new()
            .all(true)
            .filters(&filters)
            .build();

        let containers = self.client.list_containers(Some(opt)).await?;

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

    /// Stats the container
    pub async fn stats(&self, id: &Uuid) -> eyre::Result<Option<ContainerStatsResponse>> {
        let local_id = self.store.load_container_local_id(*id).await?;

        let container = ContainerId::new(local_id, *id).stats(&self.client).await?;

        Ok(container)
    }
}
