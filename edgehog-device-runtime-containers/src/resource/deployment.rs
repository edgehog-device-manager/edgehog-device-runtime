// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

use std::collections::{BTreeMap, HashSet};

use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::containers::container::{
    ContainerDeviceMapping, ContainerDeviceRequest, ContainerNetwork, ContainerVolume,
};
use edgehog_store::models::containers::deployment::DeploymentStatus;
use edgehog_store::models::containers::file_bind::{ContainerEnvFile, ContainerFileBind};
use uuid::Uuid;

use crate::properties::{
    AvailableProp, Client,
    deployment::{AvailableDeployment, DeploymentStatus as PropertyStatus},
};

use super::{Context, Resource, Result};

/// Row for a deployment
///
/// It's made of the columns: container_id, image_id, network_id, volume_id
pub(crate) type DeploymentRow = (
    SqlUuid,
    i64,
    SqlUuid,
    Option<ContainerNetwork>,
    Option<ContainerVolume>,
    Option<ContainerDeviceMapping>,
    Option<ContainerDeviceRequest>,
    Option<ContainerFileBind>,
    Option<ContainerEnvFile>,
);

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Deployment {
    pub(crate) containers: BTreeMap<i64, Uuid>,
    pub(crate) images: HashSet<Uuid>,
    pub(crate) volumes: HashSet<Uuid>,
    pub(crate) networks: HashSet<Uuid>,
    pub(crate) device_mappings: HashSet<Uuid>,
    pub(crate) device_requests: HashSet<Uuid>,
    pub(crate) file_binds: HashSet<Uuid>,
    pub(crate) env_files: HashSet<Uuid>,
}

impl From<Vec<DeploymentRow>> for Deployment {
    fn from(value: Vec<DeploymentRow>) -> Self {
        value.into_iter().fold(
            Self::default(),
            |mut acc,
             (
                container_id,
                idx,
                image_id,
                c_network,
                c_volume,
                c_device_mapping,
                c_device_request,
                c_file_bind,
                c_env_file,
            )| {
                acc.containers.insert(idx, *container_id);
                acc.images.insert(*image_id);

                if let Some(c_network) = c_network {
                    acc.networks.insert(*c_network.network_id);
                }

                if let Some(c_volume) = c_volume {
                    acc.volumes.insert(*c_volume.volume_id);
                }

                if let Some(c_device_mapping) = c_device_mapping {
                    acc.device_mappings
                        .insert(*c_device_mapping.device_mapping_id);
                }

                if let Some(c_device_request) = c_device_request {
                    acc.device_requests
                        .insert(*c_device_request.device_request_id);
                }

                if let Some(c_file_bind) = c_file_bind {
                    acc.file_binds.insert(c_file_bind.file_bind_id.0);
                }

                if let Some(c_env_file) = c_env_file {
                    acc.env_files.insert(c_env_file.env_file_id.0);
                }

                acc
            },
        )
    }
}

impl<D> Resource<D> for Deployment
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableDeployment::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Stopped)
            .await?;

        ctx.store
            .update_deployment_status(ctx.id, DeploymentStatus::Stopped)
            .await?;

        Ok(())
    }
}
