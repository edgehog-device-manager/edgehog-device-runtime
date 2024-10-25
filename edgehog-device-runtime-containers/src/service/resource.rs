// This file is part of Astarte.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;

use tracing::{debug, info, instrument, trace};

use super::{Id, Result};

use crate::{
    container::Container,
    image::Image,
    network::Network,
    properties::{
        container::{AvailableContainers, ContainerStatus},
        deployment::{AvailableDeployments, DeploymentStatus},
        image::AvailableImage,
        network::AvailableNetworks,
        volume::AvailableVolumes,
        AvailableProp, Client,
    },
    store::{Resource, StateStore},
    volume::Volume,
    Docker,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeType {
    Image(Image<String>),
    Volume(Volume<String>),
    Network(Network<String>),
    Container(Container<String>),
    Deployment,
}

impl NodeType {
    #[instrument(skip_all)]
    pub(super) async fn store<D>(
        &mut self,
        id: &Id,
        store: &mut StateStore,
        device: &D,
        deps: Vec<Id>,
    ) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        match &self {
            NodeType::Image(image) => {
                store.append(*id, image.into(), deps).await?;

                AvailableImage::new(id, false).send(device).await?;

                info!("stored image with id {id}");
            }
            NodeType::Volume(volume) => {
                store.append(*id, volume.into(), deps).await?;

                AvailableVolumes::new(id, false).send(device).await?;

                info!("stored volume with id {id}");
            }
            NodeType::Network(network) => {
                store.append(*id, network.into(), deps).await?;

                AvailableNetworks::new(id, false).send(device).await?;

                info!("stored network with id {id}");
            }
            NodeType::Container(container) => {
                store.append(*id, container.into(), deps).await?;

                AvailableContainers::new(id, ContainerStatus::Received)
                    .send(device)
                    .await?;

                info!("stored container with id {id}");
            }
            NodeType::Deployment => {
                store.append(*id, Resource::Deployment, deps).await?;

                AvailableDeployments::new(id, DeploymentStatus::Stopped)
                    .send(device)
                    .await?;

                info!("stored deployment with id {id}");
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub(super) async fn create<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        match self {
            NodeType::Image(image) => {
                image.inspect_or_create(client).await?;

                AvailableImage::new(id, true).send(device).await?;

                info!("created image with id {id}");
            }
            NodeType::Volume(volume) => {
                volume.create(client).await?;

                AvailableVolumes::new(id, true).send(device).await?;

                info!("created volume with id {id}");
            }
            NodeType::Network(network) => {
                network.inspect_or_create(client).await?;

                AvailableNetworks::new(id, true).send(device).await?;

                info!("created network with id {id}");
            }
            NodeType::Container(container) => {
                container.inspect_or_create(client).await?;

                AvailableContainers::new(id, ContainerStatus::Created)
                    .send(device)
                    .await?;

                info!("created container with id {id}");
            }
            NodeType::Deployment => {}
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub(super) async fn start<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        match self {
            NodeType::Image(_) | NodeType::Volume(_) | NodeType::Network(_) => {
                debug!("resource is up");
            }
            NodeType::Container(container) => {
                container.start(client).await?;

                AvailableContainers::new(id, ContainerStatus::Running)
                    .send(device)
                    .await?;

                info!("started container with id {id}");
            }
            NodeType::Deployment => {
                AvailableDeployments::new(id, DeploymentStatus::Started)
                    .send(device)
                    .await?;

                info!("deployment started with id {id}");
            }
        }

        info!("resource {id} started");

        Ok(())
    }

    #[instrument(skip_all)]
    pub(super) async fn stop<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        match self {
            NodeType::Image(_) | NodeType::Volume(_) | NodeType::Network(_) => {
                trace!("nothing to do for {id}");
            }
            NodeType::Container(container) => {
                let exists = container.stop(client).await?;
                debug_assert!(exists.is_some());

                AvailableContainers::new(id, ContainerStatus::Stopped)
                    .send(device)
                    .await?;
            }
            NodeType::Deployment => {
                AvailableDeployments::new(id, DeploymentStatus::Stopped)
                    .send(device)
                    .await?;
            }
        }

        info!("resource {id} stopped");

        Ok(())
    }
}

impl From<Image<String>> for NodeType {
    fn from(value: Image<String>) -> Self {
        Self::Image(value)
    }
}

impl From<Volume<String>> for NodeType {
    fn from(value: Volume<String>) -> Self {
        Self::Volume(value)
    }
}

impl From<Network<String>> for NodeType {
    fn from(value: Network<String>) -> Self {
        Self::Network(value)
    }
}

impl From<Container<String>> for NodeType {
    fn from(value: Container<String>) -> Self {
        Self::Container(value)
    }
}
