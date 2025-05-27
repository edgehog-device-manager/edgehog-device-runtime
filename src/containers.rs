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

use std::{future::Future, path::Path, time::Duration};

use async_trait::async_trait;
use edgehog_containers::{
    events::RuntimeListener,
    requests::ContainerRequest,
    service::{
        events::{EventError, ServiceHandle},
        Service, ServiceError,
    },
    store::{StateStore, StoreError},
    Docker,
};
use edgehog_store::db::Handle;
use futures::TryFutureExt;
use serde::Deserialize;
use stable_eyre::eyre::eyre;
use stable_eyre::eyre::WrapErr;
use tokio::task::JoinSet;
use tracing::error;

use crate::controller::actor::Actor;
use crate::Client;

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

/// Trait used since a FnMut is not enough to return a Future for the `service.init` case
#[async_trait]
trait TryRun {
    type Out;

    async fn run(&mut self) -> stable_eyre::Result<Self::Out>;
}

#[async_trait]
impl<F, Fut, O> TryRun for F
where
    F: FnMut() -> Fut + Send,
    Fut: Future<Output = stable_eyre::Result<O>> + Send,
{
    type Out = O;

    async fn run(&mut self) -> stable_eyre::Result<Self::Out> {
        (self)().await
    }
}

#[async_trait]
impl<D> TryRun for &mut Service<D>
where
    D: Client + Sync + Send + 'static,
{
    type Out = ();

    async fn run(&mut self) -> stable_eyre::Result<()> {
        self.init().await?;

        Ok(())
    }
}

#[async_trait]
impl<D> TryRun for &mut RuntimeListener<D>
where
    D: Client + Sync + Send + 'static,
{
    type Out = ();

    async fn run(&mut self) -> stable_eyre::Result<()> {
        self.handle_events().await?;

        Ok(())
    }
}

async fn retry<S, O>(config: &ContainersConfig, mut init: S) -> stable_eyre::Result<Option<O>>
where
    S: TryRun<Out = O>,
    O: 'static,
{
    let mut timeout = Duration::from_secs(2);

    // retry with an exponential back off
    for _ in 0..config.max_retries {
        let res = init.run().await;
        let err = match res {
            Ok(out) => return Ok(Some(out)),
            Err(err) => err,
        };

        error!(
            error = format!("{err:#}"),
            "couldn't init container service"
        );

        tokio::time::sleep(timeout).await;

        // Exponential
        timeout = Duration::from_secs(timeout.as_secs().saturating_mul(2));
    }

    error!("retried too many times, returning");

    if config.required {
        return Err(
            eyre!("couldn't initialize the container service").wrap_err(eyre!(
                "tried to start the runtime {} times",
                config.max_retries
            )),
        );
    }

    Ok(None)
}

#[cfg(not(test))]
fn spawn_listener<D>(
    config: ContainersConfig,
    store: &StateStore,
    device: &D,
    tasks: &mut JoinSet<Result<(), stable_eyre::eyre::Error>>,
) where
    D: Client + Clone + Send + Sync + 'static,
{
    use tracing::warn;

    // Use a lazy clone since the handle will only write to the database
    let store_cl = store.clone_lazy();
    let device_cl = device.clone();
    tasks.spawn(async move {
        let maybe_client = retry(&config, || Docker::connect().map_err(Into::into)).await?;
        let Some(client) = maybe_client else {
            return Ok(());
        };

        let mut listener = RuntimeListener::new(client, device_cl, store_cl);

        // TODO: the retry should have a reset time
        let should_exit = retry(&config, &mut listener).await?.is_none();
        if should_exit {
            warn!("listener retry limit reached");
        }

        Ok(())
    });
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
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let store = Handle::open(store_dir.join("state.db"))
            .await
            .map(StateStore::new)
            .map_err(|err| ServiceError::Store(StoreError::Handle(err)))?;

        // fixes an issue with features normalization when testing with `--all-features --workspace`
        #[cfg(not(test))]
        spawn_listener(config, &store, &device, tasks);

        // Use a lazy clone since the handle will only write to the database
        let store_cl = store.clone_lazy();
        let device_cl = device.clone();
        tasks.spawn(async move {
            let maybe_client = retry(&config, || Docker::connect().map_err(Into::into)).await?;
            let Some(client) = maybe_client else {
                return Ok(());
            };

            let mut service = Service::new(client, device_cl, rx, store_cl);

            let should_exit = retry(&config, &mut service).await?.is_none();
            if should_exit {
                return Ok(());
            };

            service.handle_events().await;

            Ok(())
        });

        let handle = ServiceHandle::new(device, store, tx);

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
