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
use bollard::secret::ContainerStateStatusEnum;
use edgehog_store::models::containers::container::ContainerStatus;
use tracing::{debug, warn};

use crate::{
    container::Container,
    properties::{
        container::{AvailableContainer, ContainerStatus as PropertyStatus},
        AvailableProp, Client,
    },
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone)]
pub(crate) struct ContainerResource {
    pub(crate) container: Container,
}

impl ContainerResource {
    fn new(container: Container) -> Self {
        Self { container }
    }

    async fn mark_missing<D>(&self, ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Received)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Published)
            .await?;

        Err(ResourceError::Missing {
            id: ctx.id,
            resource: "container",
        })
    }

    pub(crate) async fn start<D>(&mut self, ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        if self.container.start(ctx.client).await?.is_none() {
            return self.mark_missing(ctx).await;
        };

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Running)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Running)
            .await?;

        Ok(())
    }

    pub(crate) async fn stop<D>(&mut self, ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        if self.container.stop(ctx.client).await?.is_none() {
            return self.mark_missing(ctx).await;
        };

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Stopped)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Stopped)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl<D> Resource<D> for ContainerResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Received)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Published)
            .await?;

        Ok(())
    }
}

impl<D> Create<D> for ContainerResource
where
    D: Client + Send + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let mut container =
            ctx.store
                .find_container(ctx.id)
                .await?
                .ok_or(ResourceError::Missing {
                    id: ctx.id,
                    resource: "container",
                })?;

        let exists = container.inspect(ctx.client).await?.is_some();

        let resource = ContainerResource::new(container);
        if exists {
            ctx.store
                .update_container_local_id(ctx.id, resource.container.id.id.clone())
                .await?;

            Ok((State::Created, resource))
        } else {
            Ok((State::Missing, resource))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.container.create(ctx.client).await?;

        ctx.store
            .update_container_local_id(ctx.id, self.container.id.id.clone())
            .await?;

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Created)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Stopped)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.container.stop(ctx.client).await?;

        self.container.remove(ctx.client).await?;

        Ok(())
    }

    async fn unset(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_container(ctx.id).await?;

        Ok(())
    }

    async fn refresh(ctx: &mut Context<'_, D>) -> Result<()> {
        let Some(mut container) = ctx.store.find_container(ctx.id).await? else {
            warn!("couldn't find container");

            return Ok(());
        };

        let Some(inspect) = container.inspect(ctx.client).await? else {
            debug!("container deleted");

            AvailableContainer::new(&ctx.id)
                .send(ctx.device, PropertyStatus::Received)
                .await?;

            return Ok(());
        };

        let Some(container_state) = inspect.state.and_then(|state| state.status) else {
            warn!("couldn't find status in inspect container response");

            return Ok(());
        };

        let status = match container_state {
            ContainerStateStatusEnum::CREATED => PropertyStatus::Created,
            ContainerStateStatusEnum::RUNNING | ContainerStateStatusEnum::RESTARTING => {
                PropertyStatus::Running
            }
            ContainerStateStatusEnum::REMOVING => PropertyStatus::Received,
            ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => {
                PropertyStatus::Stopped
            }
            ContainerStateStatusEnum::PAUSED | ContainerStateStatusEnum::EMPTY => {
                debug!(%container_state, "ignoring state");

                return Ok(());
            }
        };

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, status)
            .await?;

        Ok(())
    }
}
