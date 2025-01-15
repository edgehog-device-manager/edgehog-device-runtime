// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
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
    service::{Service, ServiceError},
    Docker,
};
use serde::Deserialize;
use stable_eyre::eyre::eyre;
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

#[derive(Debug)]
pub(crate) struct ContainerService<D> {
    config: ContainersConfig,
    service: Service<D>,
}

impl<D> ContainerService<D> {
    pub(crate) async fn new(
        device: D,
        config: ContainersConfig,
        _store_dir: &Path,
    ) -> Result<Self, ServiceError> {
        let client = Docker::connect().await?;

        let service = Service::new(client, device);

        Ok(Self { config, service })
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
        let mut retries = 0usize;
        let mut timeout = Duration::from_secs(2);

        // retry with an exponential back off
        while let Err(err) = self.service.init().await {
            error!(
                error = format!("{:#}", stable_eyre::Report::new(err)),
                "couldn't init container service"
            );

            // Increase the times we tired
            retries += retries.saturating_add(1);

            if self.config.required && retries >= self.config.max_retries {
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

        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        if let Err(err) = self.service.on_event(*msg).await {
            error!(
                error = format!("{:#}", stable_eyre::Report::new(err)),
                "couldn't handle container event"
            );
        }

        Ok(())
    }
}
