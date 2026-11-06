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
use tracing::{debug, error, instrument, warn};

use crate::container::Container;
use crate::events::DatastreamEvent;
use crate::events::container::{ContainerEvent, ContainerEventStatus};
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

    pub(crate) async fn start_container<D>(&mut self, ctx: &mut Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        AvailableContainer::new(&ctx.id)
            .send_status(ctx.device, PropertyStatus::Starting)
            .await?;

        self.container
            .start(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        if !self.update_status(ctx).await? {
            return Err(ResourceError::Missing {
                id: ctx.id,
                resource: "container",
            });
        }

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Running)
            .await?;

        Ok(())
    }

    pub(crate) async fn stop_container<D>(&mut self, ctx: &mut Context<'_, D>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        AvailableContainer::new(&ctx.id)
            .send_status(ctx.device, PropertyStatus::Stopping)
            .await?;

        self.container
            .stop(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        self.update_status(ctx).await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Stopped)
            .await?;

        Ok(())
    }

    /// Returns true if the container exists
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
                .send(ctx.device, PropertyStatus::Missing)
                .await?;

            return Ok(false);
        };

        let Some(container_state) = inspect.state.as_ref().and_then(|state| state.status) else {
            warn!("couldn't find status in inspect container response");

            return Ok(false);
        };

        let (status, exists) = match container_state {
            ContainerStateStatusEnum::CREATED => (PropertyStatus::Created, true),
            ContainerStateStatusEnum::RUNNING | ContainerStateStatusEnum::RESTARTING => {
                (PropertyStatus::Running, true)
            }
            ContainerStateStatusEnum::REMOVING => (PropertyStatus::Deleting, false),
            ContainerStateStatusEnum::STOPPING => (PropertyStatus::Stopping, true),
            ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => {
                (PropertyStatus::Stopped, true)
            }
            ContainerStateStatusEnum::PAUSED | ContainerStateStatusEnum::EMPTY => {
                debug!(%container_state, "weird state");

                (PropertyStatus::Received, true)
            }
        };

        if exists {
            ctx.store
                .update_container_local_id(ctx.id, self.container.id.id.clone())
                .await?;
        }

        AvailableContainer::with_inspect(&ctx.id, inspect)
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

    pub(crate) async fn start<D>(mut ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + PropAccess + Send + Sync + 'static,
    {
        let res = Self::fetch(&mut ctx).await.and_then(|opt| {
            opt.ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "container",
            })
        });
        let (state, mut resource) = Self::handle_error(&mut ctx, res).await?;

        match state {
            State::Missing => {
                warn!("resource is missing, creating it");

                let res = resource.create(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;

                let res = resource.start_container(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;
            }
            State::Existing | State::Recreatable => {
                debug!("resource found, starting it");

                let res = resource.start_container(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;
            }
        }

        Ok(())
    }

    pub(crate) async fn stop<D>(mut ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + PropAccess + Send + Sync + 'static,
    {
        let res = Self::fetch(&mut ctx).await.and_then(|opt| {
            opt.ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "container",
            })
        });
        let (state, mut resource) = Self::handle_error(&mut ctx, res).await?;

        match state {
            State::Missing => {
                warn!("resource is missing, creating it");

                let res = resource.create(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;
            }
            State::Existing | State::Recreatable => {
                debug!("resource found, stopping it");

                let res = resource.stop_container(&mut ctx).await;
                Self::handle_error(&mut ctx, res).await?;
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

    async fn unset(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_container(ctx.id).await?;

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
            Ok(Some((State::Existing, this)))
        } else {
            Ok(Some((State::Missing, this)))
        }
    }

    #[instrument(skip_all)]
    async fn on_error(ctx: &mut Context<'_, D>, error: &(dyn std::error::Error + Send + Sync)) {
        ContainerEvent::with_error(ContainerEventStatus::Error, error)
            .send(&ctx.id, ctx.device)
            .await;

        if let Err(error) = AvailableContainer::new(&ctx.id)
            .send_status(ctx.device, PropertyStatus::Error)
            .await
        {
            error!(%error, "couldn't set container state to error");
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Creating)
            .await?;

        self.fetch_props(ctx).await?;

        self.container
            .create(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        self.update_status(ctx).await?;

        ctx.store
            .update_container_status(ctx.id, ContainerStatus::Stopped)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableContainer::new(&ctx.id)
            .send_status(ctx.device, PropertyStatus::Stopping)
            .await?;

        self.container
            .stop(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        AvailableContainer::new(&ctx.id)
            .send_status(ctx.device, PropertyStatus::Deleting)
            .await?;

        self.container
            .remove(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        Ok(())
    }
}
