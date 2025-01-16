// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

use std::{path::Path, time::Duration};

use astarte_device_sdk::Client;
use async_trait::async_trait;
use edgehog_containers::{
    requests::ContainerRequest,
    service::{
        events::{EventError, ServiceHandle},
        Service, ServiceError,
    },
    store::{StateStore, StoreError},
    Docker,
};
use edgehog_store::db::Handle;
use serde::Deserialize;
use stable_eyre::eyre::eyre;
use stable_eyre::eyre::WrapErr;
use tokio::task::JoinSet;
use tracing::error;

use crate::controller::actor::Actor;

/// Maximum number of retries for the initialization of the service
pub const MAX_INIT_RETRIES: usize = 10;

/// Configuration for the container service.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainersConfig {
    /// Flag to make the container service is required
    #[serde(default)]
    required: bool,
    /// Maximum number of retries for the initialization of the service
    #[serde(default = "ContainersConfig::default_max_retries")]
    max_retries: usize,
}

impl ContainersConfig {
    const fn default_max_retries() -> usize {
        MAX_INIT_RETRIES
    }
}

impl Default for ContainersConfig {
    fn default() -> Self {
        Self {
            required: false,
            max_retries: Self::default_max_retries(),
        }
    }
}

async fn handle_events<D>(
    mut service: Service<D>,
    config: ContainersConfig,
) -> stable_eyre::Result<()>
where
    D: Client + Send + Sync + 'static,
{
    let mut retries = 0usize;
    let mut timeout = Duration::from_secs(2);

    // retry with an exponential back off
    while let Err(err) = service.init().await {
        error!(
            error = format!("{:#}", stable_eyre::Report::new(err)),
            "couldn't init container service"
        );

        // Increase the times we tried
        retries += retries.saturating_add(1);

        if config.required && retries >= config.max_retries {
            return Err(
                eyre!("couldn't initialize the container service").wrap_err(eyre!(
                    "tried to start the runtime {retries} times, but reached the maximum"
                )),
            );
        }

        tokio::time::sleep(timeout).await;

        // Exponential
        timeout = Duration::from_secs(timeout.as_secs().saturating_mul(2));
    }

    service.handle_events().await;

    Ok(())
}

#[derive(Debug)]
pub(crate) struct ContainerService<D> {
    handle: ServiceHandle<D>,
}

impl<D> ContainerService<D> {
    pub(crate) async fn new(
        device: D,
        config: ContainersConfig,
        store_dir: &Path,
        tasks: &mut JoinSet<stable_eyre::Result<()>>,
    ) -> Result<Self, ServiceError>
    where
        D: Client + Clone + Send + Sync + 'static,
    {
        // TODO: run this logic with the retry
        let client = Docker::connect().await?;

        let handle = Handle::open(store_dir.join("state.db"))
            .await
            .map_err(|err| ServiceError::Store(StoreError::Handle(err)))?;
        let store = StateStore::new(handle);

        let (service, handle) = Service::new(client, store, device);

        tasks.spawn(handle_events(service, config));

        Ok(Self { handle })
    }
}

#[async_trait]
impl<D> Actor for ContainerService<D>
where
    D: Client + Send + Sync + 'static,
{
    type Msg = Box<ContainerRequest>;

    fn task() -> &'static str {
        "containers"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        let res = self.handle.on_event(*msg).await;
        match res {
            Ok(()) => {}
            Err(EventError::Disconnected) => {
                return res.wrap_err("couldn't handle container event")
            }
        }

        Ok(())
    }
}
