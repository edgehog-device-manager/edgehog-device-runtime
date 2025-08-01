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

use astarte_device_sdk::{Client, FromEvent};
use color_eyre::eyre::bail;
use edgehog_containers::{
    events::RuntimeListener,
    requests::ContainerRequest,
    service::{events::ServiceHandle, Service},
    stats::StatsMonitor,
    store::StateStore,
    Docker,
};
use edgehog_store::db::Handle;
use tokio::task::JoinSet;
use tracing::{error, info};

async fn receive_events<D>(device: D, mut handle: ServiceHandle<D>) -> color_eyre::Result<()>
where
    D: Client + Send + Sync + 'static,
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
    D: Client + Clone + Send + Sync + 'static,
{
    service.init().await?;

    service.handle_events().await;

    Ok(())
}

async fn runtime_listen<D>(mut listener: RuntimeListener<D>) -> color_eyre::Result<()>
where
    D: Client + Send + Sync + 'static,
{
    listener.handle_events().await?;

    Ok(())
}

async fn container_stats<D>(mut stats: StatsMonitor<D>) -> color_eyre::Result<()>
where
    D: Client + Send + Sync + 'static,
{
    let mut interval = tokio::time::interval(Duration::from_secs(10));

    loop {
        stats.gather().await?;

        interval.tick().await;
    }
}

pub async fn receive<D>(device: D, store_path: &Path) -> color_eyre::Result<()>
where
    D: Client + Send + Sync + 'static,
{
    let client = Docker::connect().await?;

    let handle = Handle::open(store_path.join("state.db")).await?;
    let store = StateStore::new(handle);

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    let listener = RuntimeListener::new(client.clone(), device.clone(), store.clone_lazy());
    let stats = StatsMonitor::new(client.clone(), device.clone(), store.clone_lazy());
    let handle = ServiceHandle::new(device.clone(), store.clone_lazy(), tx);
    let service = Service::new(client, device.clone(), rx, store);

    let mut tasks = JoinSet::new();

    tasks.spawn(handle_events(service));
    tasks.spawn(runtime_listen(listener));
    tasks.spawn(container_stats(stats));
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
