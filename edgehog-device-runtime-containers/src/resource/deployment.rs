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

use std::collections::HashSet;

use async_trait::async_trait;
use edgehog_store::{
    conversions::SqlUuid,
    models::containers::{
        container::{ContainerDeviceMapping, ContainerNetwork, ContainerVolume},
        deployment::DeploymentStatus,
    },
};
use uuid::Uuid;

use crate::properties::{
    deployment::{AvailableDeployment, DeploymentStatus as PropertyStatus},
    AvailableProp, Client,
};

use super::{Context, Resource, Result};

/// Row for a deployment
///
/// It's made of the columns: container_id, image_id, network_id, volume_id
pub(crate) type DeploymentRow = (
    SqlUuid,
    SqlUuid,
    Option<ContainerNetwork>,
    Option<ContainerVolume>,
    Option<ContainerDeviceMapping>,
);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Deployment {
    pub(crate) containers: HashSet<Uuid>,
    pub(crate) images: HashSet<Uuid>,
    pub(crate) volumes: HashSet<Uuid>,
    pub(crate) networks: HashSet<Uuid>,
    pub(crate) device_mapping: HashSet<Uuid>,
}

impl From<Vec<DeploymentRow>> for Deployment {
    fn from(value: Vec<DeploymentRow>) -> Self {
        value.into_iter().fold(
            Self::default(),
            |mut acc, (container_id, image_id, c_network, c_volume, c_device_mapping)| {
                acc.containers.insert(*container_id);
                acc.images.insert(*image_id);

                if let Some(c_network) = c_network {
                    acc.networks.insert(*c_network.network_id);
                }

                if let Some(c_volume) = c_volume {
                    acc.volumes.insert(*c_volume.volume_id);
                }

                if let Some(c_device_mapping) = c_device_mapping {
                    acc.device_mapping
                        .insert(*c_device_mapping.device_mapping_id);
                }

                acc
            },
        )
    }
}

#[async_trait]
impl<D> Resource<D> for Deployment
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableDeployment::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Stopped)
            .await?;

        ctx.store
            .update_deployment_status(ctx.id, DeploymentStatus::Stopped)
            .await?;

        Ok(())
    }
}
