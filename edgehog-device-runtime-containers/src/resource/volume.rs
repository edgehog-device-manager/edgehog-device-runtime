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

use async_trait::async_trait;
use edgehog_store::models::containers::volume::VolumeStatus;

use crate::{
    properties::{volume::AvailableVolume, AvailableProp, Client},
    volume::Volume,
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone)]
pub(crate) struct VolumeResource {
    pub(crate) volume: Volume,
}

impl VolumeResource {
    pub(crate) fn new(volume: Volume) -> Self {
        Self { volume }
    }
}

#[async_trait]
impl<D> Resource<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id)
            .send(ctx.device, false)
            .await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Published)
            .await?;

        Ok(())
    }
}

impl<D> Create<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let resource = ctx
            .store
            .find_volume(ctx.id)
            .await?
            .ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "volume",
            })?;

        let exists = resource.volume.inspect(ctx.client).await?.is_some();

        if exists {
            Ok((State::Created, resource))
        } else {
            Ok((State::Missing, resource))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.volume.create(ctx.client).await?;

        AvailableVolume::new(&ctx.id).send(ctx.device, true).await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Created)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.volume.remove(ctx.client).await?;

        Ok(())
    }

    async fn unset(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_volume(ctx.id).await?;

        Ok(())
    }
}
