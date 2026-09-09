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

use std::ops::ControlFlow;

use astarte_device_sdk::properties::PropAccess;
use bollard::models::ContainerStateStatusEnum;
use edgehog_store::models::containers::container::ContainerStatus;
use tracing::{debug, warn};

use crate::container::Container;
use crate::properties::container::{AvailableContainer, ContainerStatus as PropertyStatus};
use crate::properties::{AvailableProp, Client};
use crate::resource::file_bind::FileBindResource;

use super::env_file::EnvFileResource;
use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ContainerResource {
    pub(crate) container: Container,
    pub(crate) file_binds: Vec<FileBindResource<'static>>,
    pub(crate) env_files: Vec<EnvFileResource<'static>>,
    /// Flag to only once fetch the props
    ///
    /// We need to query the props only when creating the container.
    props_fetched: bool,
}

impl ContainerResource {
    pub(crate) fn new(
        container: Container,
        file_binds: Vec<FileBindResource<'static>>,
        env_files: Vec<EnvFileResource<'static>>,
    ) -> Self {
        Self {
            container,
            file_binds,
            env_files,
            props_fetched: false,
        }
    }

    async fn mark_missing<D>(&self, ctx: Context<'_, D>) -> Result<std::convert::Infallible>
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
        let container = self
            .container
            .start(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        if container.is_none() {
            return self.mark_missing(ctx).await.map(|_| ());
        }

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
        let container = self
            .container
            .stop(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        if container.is_none() {
            return self.mark_missing(ctx).await.map(|_| ());
        }

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Stopped)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Stopped)
            .await?;

        Ok(())
    }

    async fn update_status<D>(&mut self, ctx: &mut Context<'_, D>) -> Result<bool>
    where
        D: Client + Send + Sync + 'static,
    {
        let resp = self
            .container
            .inspect(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        let Some(inspect) = resp else {
            debug!("container deleted");

            AvailableContainer::new(&ctx.id)
                .send(ctx.device, PropertyStatus::Received)
                .await?;

            return Ok(false);
        };

        let Some(container_state) = inspect.state.and_then(|state| state.status) else {
            warn!("couldn't find status in inspect container response");

            return Ok(false);
        };

        let (status, exists) = match container_state {
            ContainerStateStatusEnum::CREATED => (PropertyStatus::Created, true),
            ContainerStateStatusEnum::RUNNING | ContainerStateStatusEnum::RESTARTING => {
                (PropertyStatus::Running, true)
            }
            ContainerStateStatusEnum::REMOVING => (PropertyStatus::Received, false),
            ContainerStateStatusEnum::STOPPING
            | ContainerStateStatusEnum::EXITED
            | ContainerStateStatusEnum::DEAD => (PropertyStatus::Stopped, true),
            ContainerStateStatusEnum::PAUSED | ContainerStateStatusEnum::EMPTY => {
                debug!(%container_state, "weird state");

                (PropertyStatus::Created, true)
            }
        };

        AvailableContainer::new(&ctx.id)
            .send(ctx.device, status)
            .await?;

        Ok(exists)
    }

    async fn fetch_props<D>(&mut self, ctx: &Context<'_, D>) -> Result<()>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        if self.props_fetched {
            return Ok(());
        }

        self.props_fetched = true;

        for file_bind in &self.file_binds {
            let bind = file_bind.to_container_bind(ctx).await?;
            self.container.add_binds(std::iter::once(bind));
        }

        for env_file in &self.env_files {
            let mut reader = env_file.open(ctx).await?;

            while let ControlFlow::Continue(opt) = reader
                .next_env()
                .await
                .map_err(|err| err.into_resource_err(env_file.inner.id.0))?
            {
                if let Some(env) = opt {
                    self.container
                        .add_env_vars(std::iter::once(env.to_string()));
                }
            }
        }

        Ok(())
    }
}

impl<D> Resource<D> for ContainerResource
where
    D: Client + PropAccess + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Received)
            .await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Published)
            .await?;

        Self::fetch(ctx).await?;

        Ok(())
    }
}

impl<D> Create<D> for ContainerResource
where
    D: Client + PropAccess + Send + Sync + 'static,
{
    const RESOURCE_NAME: &str = "container";

    async fn fetch(ctx: &mut Context<'_, D>) -> Result<Option<(State, Self)>> {
        let Some(mut this) = ctx.store.find_container(ctx.id).await? else {
            return Ok(None);
        };

        let exists = this.update_status(ctx).await?;

        if exists {
            ctx.store
                .update_container_local_id(ctx.id, this.container.id.id.clone())
                .await?;

            Ok(Some((State::Created, this)))
        } else {
            Ok(Some((State::Missing, this)))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.fetch_props(ctx).await?;

        self.container
            .create(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

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
        self.container
            .stop(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        self.container
            .remove(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        Ok(())
    }

    async fn unset(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_container(ctx.id).await?;

        Ok(())
    }
}
