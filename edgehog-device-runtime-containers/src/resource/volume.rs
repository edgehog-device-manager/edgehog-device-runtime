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

use async_trait::async_trait;
use edgehog_store::models::containers::volume::VolumeStatus;

use crate::{
    properties::{volume::AvailableVolume, AvailableProp, Client},
    volume::Volume,
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[async_trait]
impl<D> Resource<D> for Volume
where
    D: Client + Sync + 'static,
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

#[async_trait]
impl<D> Create<D> for Volume
where
    D: Client + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let volume =
            ctx.store
                .volume_for_container(ctx.id)
                .await?
                .ok_or(ResourceError::Missing {
                    id: ctx.id,
                    resource: "volume",
                })?;

        let exists = volume.inspect(ctx.client).await?.is_some();

        if exists {
            Ok((State::Created, volume))
        } else {
            Ok((State::Missing, volume))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.inspect_or_create(ctx.client).await?;

        AvailableVolume::new(&ctx.id).send(ctx.device, true).await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Created)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.remove(ctx.client).await?;

        AvailableVolume::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_volume(ctx.id).await?;

        Ok(())
    }
}
