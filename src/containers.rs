// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

use std::sync::Arc;

use edgehog_containers::requests::ContainerRequest;
use edgehog_containers::service::Service;
use edgehog_containers::service::events::{ContainerEvent, EventError, ServiceHandle};
use edgehog_containers::store::StateStore;
use edgehog_containers::{Docker, events::RuntimeListener};
use edgehog_store::db;
use eyre::WrapErr;
use eyre::eyre;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub(crate) use edgehog_containers::local::MockContainerHandle as ContainerHandle;
    } else {
        pub(crate) use edgehog_containers::local::ContainerHandle;
    }
}

use crate::Client;
use crate::controller::EVENT_BUFFER;
use crate::controller::actor::Actor;

/// Maximum number of retries for the initialization of the service
pub const MAX_INIT_RETRIES: usize = 10;
/// Max number of events
pub const CHANNEL_SIZE: usize = 64;

/// Configuration for the container service.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ContainersConfig {
    #[serde(default = "ContainersConfig::default_enabled")]
    pub(crate) enabled: bool,
    /// Flag to make the container service is required
    #[serde(default)]
    required: bool,
    /// Maximum number of retries for the initialization of the service
    #[serde(default = "ContainersConfig::default_max_retries")]
    max_retries: usize,
}

impl ContainersConfig {
    const fn default_enabled() -> bool {
        true
    }
    const fn default_max_retries() -> usize {
        MAX_INIT_RETRIES
    }
}

impl Default for ContainersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            required: false,
            max_retries: Self::default_max_retries(),
        }
    }
}

// Setups the containers
pub(crate) async fn setup<D>(
    tasks: &mut JoinSet<eyre::Result<()>>,
    config: ContainersConfig,
    device: &D,
    store: &db::Handle,
    cancel: CancellationToken,
) -> eyre::Result<Option<(mpsc::Sender<Box<ContainerRequest>>, Arc<ContainerHandle>)>>
where
    D: Client + Clone + Send + Sync + 'static,
{
    // Try to connect to docker or fail.
    let client = match Docker::connect().await {
        Ok(client) => client,
        Err(error) => {
            error!(%error, "couldn't connect to container runtime");

            if config.required {
                return Err(eyre!("container runtime is required, but couldn't connect"));
            } else {
                debug!("container runtime not required in config");

                return Ok(None);
            }
        }
    };

    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let (contaienr_tx, container_rx) = tokio::sync::mpsc::channel(CHANNEL_SIZE);

    let store = StateStore::new(store.clone());

    // fixes an issue with features normalization when testing with `--all-features --workspace`
    spawn_listener(
        tasks,
        &config,
        &store,
        &client,
        contaienr_tx.clone(),
        &cancel,
    );

    let service = ContainerService {
        config,
        service: Service::new(client.clone(), device.clone(), store.clone()),
    };
    let receiver = ContainerReceiver {
        handle: ServiceHandle::new(device.clone(), store.clone(), contaienr_tx),
    };

    tasks.spawn(service.run(container_rx, cancel.clone()));
    tasks.spawn(receiver.run(rx, cancel.clone()));

    let container_handle = Arc::new(ContainerHandle::new(client, store));

    Ok(Some((tx, container_handle)))
}

fn spawn_listener(
    tasks: &mut JoinSet<Result<(), eyre::Error>>,
    config: &ContainersConfig,
    store: &StateStore,
    client: &Docker,
    tx: tokio::sync::mpsc::Sender<ContainerEvent>,
    cancel: &CancellationToken,
) {
    if cfg!(test) {
        return;
    }

    let mut listener = RuntimeListener::new(client.clone(), store.clone(), tx);

    let cancel = cancel.clone();
    let max_retries = config.max_retries;
    let required = config.required;

    tasks.spawn(async move {
        let mut retries = 0;

        while let Some(res) = cancel.run_until_cancelled(listener.handle_events()).await {
            if let Err(error) = res {
                error!(%error, "couldn't handle container listener events");

                retries += 1;
            }

            if retries >= max_retries {
                if required {
                    return Err(eyre!("container listener max retries reached"));
                } else {
                    break;
                }
            }
        }

        Ok(())
    });
}

#[derive(Debug)]
pub(crate) struct ContainerReceiver<D> {
    handle: ServiceHandle<D>,
}

impl<D> Actor for ContainerReceiver<D>
where
    D: Client + Send + Sync + 'static,
{
    type Msg = Box<ContainerRequest>;

    fn task() -> &'static str {
        "container_receiver"
    }

    async fn init(&mut self) -> eyre::Result<()> {
        Ok(())
    }

    async fn handle(&mut self, cancel: &CancellationToken, msg: Self::Msg) -> eyre::Result<()> {
        let res = self.handle.on_event(cancel, *msg).await;

        match res {
            Ok(()) => {}
            Err(EventError::Disconnected) => {
                return res.wrap_err("couldn't handle container event");
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ContainerService<D> {
    config: ContainersConfig,
    service: Service<D>,
}

impl<D> Actor for ContainerService<D>
where
    D: Client + Send + Sync + 'static,
{
    type Msg = ContainerEvent;

    fn task() -> &'static str {
        "container_service"
    }

    fn required(&self) -> bool {
        self.config.required
    }

    async fn init(&mut self) -> eyre::Result<()> {
        self.service.initialize().await?;

        Ok(())
    }

    async fn handle(&mut self, _cancel: &CancellationToken, msg: Self::Msg) -> eyre::Result<()> {
        self.service.on_event(msg).await;

        Ok(())
    }
}
