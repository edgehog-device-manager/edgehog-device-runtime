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

use std::{fmt::Debug, path::Path};

use astarte_device_sdk::{Client, FromEvent};
use color_eyre::eyre::bail;
use edgehog_containers::{
    requests::ContainerRequest,
    service::{events::ServiceHandle, Service},
    store::StateStore,
    Docker,
};
use edgehog_store::db::Handle;
use tokio::task::JoinSet;
use tracing::{error, info};

async fn receive_events<D>(device: D, handle: ServiceHandle<D>) -> color_eyre::Result<()>
where
    D: Debug + Client + Send + Sync + 'static,
{
    loop {
        let event = device.recv().await?;

        match ContainerRequest::from_event(event) {
            Ok(req) => {
                handle.on_event(req).await.inspect_err(|err| {
                    error!(error = format!("{:#}", err), "couldn't handle the event");
                })?;
            }
            Err(err) => {
                error!(
                    error = format!("{:#}", color_eyre::Report::new(err)),
                    "couldn't parse the event"
                );

                bail!("invalid event received");
            }
        }
    }
}

async fn handle_events<D>(mut service: Service<D>) -> color_eyre::Result<()>
where
    D: Debug + Client + Clone + Send + Sync + 'static,
{
    service.init().await?;

    service.handle_events().await;

    Ok(())
}

pub async fn receive<D>(device: D, store_path: &Path) -> color_eyre::Result<()>
where
    D: Debug + Client + Clone + Send + Sync + 'static,
{
    let client = Docker::connect().await?;

    let handle = Handle::open(store_path.join("state.db")).await?;
    let store = StateStore::new(handle);

    let (service, handle) = Service::new(client, store, device.clone());

    let mut tasks = JoinSet::new();

    tasks.spawn(handle_events(service));
    tasks.spawn(receive_events(device, handle));

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok(())) => {
                info!("task exited");

                tasks.abort_all();
            }
            Ok(Err(err)) => {
                error!(error = format!("{:#}", err), "joined with error");

                return Err(err);
            }
            Err(err) if err.is_cancelled() => {}
            Err(err) => {
                error!(
                    error = format!("{:#}", color_eyre::Report::new(err)),
                    "task panicked"
                );

                bail!("task panicked");
            }
        }
    }

    Ok(())
}
