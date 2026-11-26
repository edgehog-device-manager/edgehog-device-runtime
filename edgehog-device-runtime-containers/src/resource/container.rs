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
use std::time::Duration;

use astarte_device_sdk::properties::PropAccess;
use bollard::models::{ContainerInspectResponse, ContainerStateStatusEnum, HealthStatusEnum};
use edgehog_store::models::containers::container::ContainerStatus;
use tracing::{debug, error, instrument, trace, warn};

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

        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;

            let Some(info) = self.update_status(ctx).await? else {
                return Err(ResourceError::Missing {
                    id: ctx.id,
                    resource: "container",
                });
            };

            let readiness = Readiness::new(info);

            debug!(?readiness);

            match readiness {
                Some(Readiness::Ready | Readiness::Exited) | None => break,
                Some(Readiness::Pending) => {
                    trace!("waiting for pending container");
                }
                Some(Readiness::Failed) => {
                    return Err(ResourceError::Failed {
                        id: ctx.id,
                        resource: "container",
                        ctx: "failed to start",
                    });
                }
            }
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
    async fn update_status<D>(
        &mut self,
        ctx: &mut Context<'_, D>,
    ) -> Result<Option<ContainerInspectResponse>>
    where
        D: Client + Send + Sync + 'static,
    {
        let resp = self
            .container
            .inspect(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        ctx.store
            .update_container_local_id(ctx.id, self.container.id.id.clone())
            .await?;

        let Some(inspect) = resp else {
            debug!("container deleted");

            AvailableContainer::new(&ctx.id)
                .send(ctx.device, PropertyStatus::Missing)
                .await?;

            return Ok(None);
        };

        let Some(container_state) = inspect.state.as_ref().and_then(|state| state.status) else {
            warn!("couldn't find status in inspect container response");

            return Ok(None);
        };

        let status = match container_state {
            ContainerStateStatusEnum::RUNNING | ContainerStateStatusEnum::RESTARTING => {
                PropertyStatus::Running
            }
            ContainerStateStatusEnum::STOPPING => PropertyStatus::Stopping,
            ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => {
                PropertyStatus::Stopped
            }
            ContainerStateStatusEnum::CREATED => PropertyStatus::Created,
            ContainerStateStatusEnum::REMOVING => PropertyStatus::Deleting,
            ContainerStateStatusEnum::PAUSED | ContainerStateStatusEnum::EMPTY => {
                debug!(%container_state, "weird state");

                PropertyStatus::Created
            }
        };

        debug!(%status);

        AvailableContainer::with_inspect(&ctx.id, &inspect)
            .send(ctx.device, status)
            .await?;

        Ok(Some(inspect))
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

    /// Start a created container.
    pub(crate) async fn start<D>(&mut self, mut ctx: Context<'_, D>) -> Result<()>
    where
        D: Client + PropAccess + Send + Sync + 'static,
    {
        let res = self.start_container(&mut ctx).await;
        Self::handle_error(&mut ctx, res).await?;

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

        let exists = this.update_status(ctx).await?.is_some();

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

/// Container readiness status
#[derive(Debug, PartialEq)]
enum Readiness {
    Ready,
    Exited,
    Pending,
    Failed,
}

impl Readiness {
    fn new(info: ContainerInspectResponse) -> Option<Self> {
        let state = info.state?;
        let status = state.status?;

        let has_healthcheck = info
            .config
            .as_ref()
            .and_then(|c| c.healthcheck.as_ref())
            .and_then(|hc| hc.test.as_ref())
            .is_some_and(|test| test.first().is_some_and(|test| test != "NONE"));
        let has_max_retries = info
            .host_config
            .as_ref()
            .and_then(|host_config| host_config.restart_policy.as_ref()?.maximum_retry_count)
            .is_some();

        // Check both the max retry and health check to determine if we need to wait for the container
        let health_status = if has_healthcheck && has_max_retries {
            state.health.as_ref().and_then(|health| health.status)
        } else {
            None
        };

        match status {
            ContainerStateStatusEnum::EMPTY
            | ContainerStateStatusEnum::CREATED
            | ContainerStateStatusEnum::PAUSED
            | ContainerStateStatusEnum::REMOVING => None,
            ContainerStateStatusEnum::STOPPING | ContainerStateStatusEnum::RESTARTING => {
                // Wait for next state
                Some(Readiness::Pending)
            }
            ContainerStateStatusEnum::RUNNING => match health_status {
                Some(HealthStatusEnum::HEALTHY) => Some(Readiness::Ready),
                Some(HealthStatusEnum::UNHEALTHY) => Some(Readiness::Failed),
                None | Some(HealthStatusEnum::EMPTY) | Some(HealthStatusEnum::NONE) => {
                    debug!("health check status missing");

                    Some(Readiness::Ready)
                }
                Some(HealthStatusEnum::STARTING) => Some(Readiness::Pending),
            },
            ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::DEAD => {
                if state.exit_code.is_some_and(|code| code == 0) {
                    Some(Readiness::Exited)
                } else {
                    Some(Readiness::Failed)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bollard::models::{
        ContainerConfig, ContainerState, Health, HealthConfig, HostConfig, RestartPolicy,
    };
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(None, None)]
    #[case(Some(ContainerState::default()), None)]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::EMPTY),
            ..Default::default()
        }),
        None
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::CREATED),
            ..Default::default()
        }),
        None
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::PAUSED),
            ..Default::default()
        }),
        None
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::REMOVING),
            ..Default::default()
        }),
        None
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::STOPPING),
            ..Default::default()
        }),
        Some(Readiness::Pending)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::RESTARTING),
            ..Default::default()
        }),
        Some(Readiness::Pending)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::RUNNING),
            ..Default::default()
        }),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::EXITED),
            exit_code: Some(0),
            ..Default::default()
        }),
        Some(Readiness::Exited)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::EXITED),
            exit_code: Some(1),
            ..Default::default()
        }),
        Some(Readiness::Failed)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::EXITED),
            exit_code: None,
            ..Default::default()
        }),
        Some(Readiness::Failed)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::DEAD),
            exit_code: Some(0),
            ..Default::default()
        }),
        Some(Readiness::Exited)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::DEAD),
            exit_code: Some(1),
            ..Default::default()
        }),
        Some(Readiness::Failed)
    )]
    #[case(
        Some(ContainerState {
            status: Some(ContainerStateStatusEnum::DEAD),
            exit_code: None,
            ..Default::default()
        }),
        Some(Readiness::Failed)
    )]
    fn should_check_readiness_from_state(
        #[case] state: Option<ContainerState>,
        #[case] expected: Option<Readiness>,
    ) {
        let info = ContainerInspectResponse {
            state,
            ..Default::default()
        };

        assert_eq!(Readiness::new(info), expected);
    }

    #[rstest]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 0".to_string()]),
        Some(3),
        Some(HealthStatusEnum::HEALTHY),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 1".to_string()]),
        Some(3),
        Some(HealthStatusEnum::UNHEALTHY),
        Some(Readiness::Failed)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 0".to_string()]),
        Some(3),
        Some(HealthStatusEnum::STARTING),
        Some(Readiness::Pending)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 0".to_string()]),
        Some(3),
        Some(HealthStatusEnum::EMPTY),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 0".to_string()]),
        Some(3),
        Some(HealthStatusEnum::NONE),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 0".to_string()]),
        Some(3),
        None,
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec!["NONE".to_string()]),
        Some(3),
        Some(HealthStatusEnum::UNHEALTHY),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec![]),
        Some(3),
        Some(HealthStatusEnum::UNHEALTHY),
        Some(Readiness::Ready)
    )]
    #[case(
        None,
        Some(3),
        Some(HealthStatusEnum::UNHEALTHY),
        Some(Readiness::Ready)
    )]
    #[case(
        Some(vec!["CMD-SHELL".to_string(), "exit 1".to_string()]),
        None,
        Some(HealthStatusEnum::UNHEALTHY),
        Some(Readiness::Ready)
    )]
    fn should_check_readiness_with_healthcheck(
        #[case] test: Option<Vec<String>>,
        #[case] maximum_retry_count: Option<i64>,
        #[case] health_status: Option<HealthStatusEnum>,
        #[case] expected: Option<Readiness>,
    ) {
        let info = ContainerInspectResponse {
            config: Some(ContainerConfig {
                healthcheck: Some(HealthConfig {
                    test,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            host_config: Some(HostConfig {
                restart_policy: Some(RestartPolicy {
                    maximum_retry_count,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            state: Some(ContainerState {
                status: Some(ContainerStateStatusEnum::RUNNING),
                health: Some(Health {
                    status: health_status,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert_eq!(Readiness::new(info), expected);
    }
}
