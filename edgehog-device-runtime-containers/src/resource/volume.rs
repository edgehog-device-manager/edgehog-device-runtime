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

use edgehog_store::models::containers::volume::{ReclaimPolicy, VolumeStatus};
use tracing::{debug, error, instrument};

use crate::{
    properties::{
        AvailableProp, Client,
        volume::{AvailableVolume, AvailableVolumeStatus},
    },
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

    fn should_retain(&self) -> bool {
        self.volume.reclaim_policy == ReclaimPolicy::Retain
    }

    #[instrument(skip_all, fields(id = %ctx.id))]
    pub(crate) async fn reclaim<D>(mut ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        let (state, mut resource) = Self::fetch(&mut ctx).await.and_then(|opt| {
            opt.ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: <Self as Create<D>>::RESOURCE_NAME,
            })
        })?;

        if resource.should_retain() {
            debug!("retaining volume");
            return Ok(());
        }

        debug!("deleting volume");

        let result = match state {
            State::Missing => {
                debug!("resource already missing");

                return Ok(());
            }
            State::Created => {
                debug!("resource found, deleting it");

                resource.delete(&mut ctx).await
            }
        };

        if let Err(error) = result {
            Self::error(&mut ctx, &error).await;

            return Err(error);
        }

        resource.unset(&mut ctx).await?;

        Ok(())
    }
}

impl<D> Resource<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Missing)
            .await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Published)
            .await?;

        Self::fetch(ctx).await?;

        Ok(())
    }
}

impl<D> Create<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    const RESOURCE_NAME: &str = "volume";

    async fn error<E>(ctx: &mut Context<'_, D>, error: &E)
    where
        E: std::error::Error + Send + Sync,
    {
        error!(%error, "error in volume resource");

        let result = AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Error)
            .await;

        if let Err(error) = result {
            error!(%error, "error while updating resource status to error");
        }
    }

    async fn fetch(ctx: &mut Context<'_, D>) -> Result<Option<(State, Self)>> {
        let Some(resource) = ctx.store.find_volume(ctx.id).await? else {
            return Ok(None);
        };

        let status = resource
            .volume
            .inspect(ctx.client)
            .await
            .map_err(ResourceError::docker)?
            .map(|_| AvailableVolumeStatus::Present)
            .unwrap_or(AvailableVolumeStatus::Missing);

        AvailableVolume::new(&ctx.id)
            .send(ctx.device, status)
            .await?;

        if status.is_present() {
            Ok(Some((State::Created, resource)))
        } else {
            Ok(Some((State::Missing, resource)))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.volume
            .create(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Present)
            .await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Created)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Deleting)
            .await?;

        self.volume
            .remove(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        Ok(())
    }

    async fn unset(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_volume(ctx.id).await?;

        Ok(())
    }
}
