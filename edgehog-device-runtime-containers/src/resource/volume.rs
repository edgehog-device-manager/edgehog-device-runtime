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
    events::{
        DatastreamEvent,
        volume::{VolumeEvent, VolumeEventStatus},
    },
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

    #[instrument(skip_all)]
    pub(crate) async fn reclaim<D>(mut ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        let res = Self::fetch(&mut ctx).await.and_then(|opt| {
            opt.ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: <Self as Create<D>>::RESOURCE_NAME,
            })
        });

        let (state, mut resource) = Self::handle_error(&mut ctx, res).await?;

        match state {
            State::Existing | State::Recreatable if resource.should_retain() => {
                debug!("retaining volume");

                let res = AvailableVolume::new(&ctx.id)
                    .send_status(ctx.device, AvailableVolumeStatus::Present)
                    .await
                    .map_err(ResourceError::from);

                return Self::handle_error(&mut ctx, res).await;
            }
            State::Missing => {
                debug!("resource already missing");
            }
            State::Existing | State::Recreatable => {
                debug!("resource found, no retain policy");

                let res = resource.delete(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;
            }
        };

        Self::unset(&mut ctx).await
    }
}

impl<D> Resource<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Received)
            .await?;

        ctx.store
            .update_volume_status(ctx.id, VolumeStatus::Published)
            .await?;

        Self::fetch(ctx).await?;

        Ok(())
    }

    async fn unset(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableVolume::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_volume(ctx.id).await?;

        Ok(())
    }
}

impl<D> Create<D> for VolumeResource
where
    D: Client + Send + Sync + 'static,
{
    const RESOURCE_NAME: &str = "volume";

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
            Ok(Some((State::Existing, resource)))
        } else {
            Ok(Some((State::Missing, resource)))
        }
    }

    #[instrument(skip_all)]
    async fn on_error(ctx: &mut Context<'_, D>, error: &(dyn std::error::Error + Send + Sync)) {
        VolumeEvent::with_error(VolumeEventStatus::Error, error)
            .send(&ctx.id, ctx.device)
            .await;

        let result = AvailableVolume::new(&ctx.id)
            .send(ctx.device, AvailableVolumeStatus::Error)
            .await;

        if let Err(error) = result {
            error!(%error, "error while updating resource status to error");
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
}
