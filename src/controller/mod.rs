// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

use actor::Actor;
use astarte_device_sdk::FromEvent;
use astarte_device_sdk::client::RecvError;
use astarte_device_sdk::prelude::PropAccess;
use eyre::{Context, Report};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, trace};

use crate::commands::execute_command;
use crate::file_transfer::FileTransfer;
use crate::file_transfer::interface::FileTransferEvent;
use crate::telemetry::Telemetry;
use crate::telemetry::event::TelemetryEvent;
use crate::{Client, DeviceManagerOptions};

#[cfg(all(feature = "zbus", target_os = "linux"))]
use crate::led_behavior::{LedBlink, LedEvent};
#[cfg(all(feature = "zbus", target_os = "linux"))]
use crate::ota::ota_handler::OtaHandler;

use self::event::RuntimeEvent;

pub mod actor;
pub mod event;

const EVENT_BUFFER: usize = 8;

#[derive(Debug)]
pub struct Runtime<T> {
    client: T,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    file_transfer: mpsc::Sender<FileTransferEvent>,
    #[cfg(feature = "containers")]
    containers_tx: mpsc::UnboundedSender<Box<edgehog_containers::requests::ContainerRequest>>,
    #[cfg(feature = "forwarder")]
    forwarder: crate::forwarder::Forwarder<T>,
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    led_tx: mpsc::Sender<LedEvent>,
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    ota_handler: OtaHandler,
}

impl<C> Runtime<C> {
    pub async fn new(
        tasks: &mut JoinSet<eyre::Result<()>>,
        opts: DeviceManagerOptions,
        client: C,
        cancel: CancellationToken,
    ) -> eyre::Result<Self>
    where
        C: Client + PropAccess + Send + Sync + 'static,
    {
        #[cfg(all(feature = "systemd", target_os = "linux"))]
        crate::systemd_wrapper::systemd_notify_status("Initializing");

        info!("Initializing");

        // needed for warning
        #[cfg(not(feature = "service"))]
        let _ = cancel;

        #[cfg(feature = "containers")]
        let store = Self::store(&opts.store_directory)
            .await
            .wrap_err("couldn't connect to container store")?;
        #[cfg(feature = "containers")]
        let container_handle = std::sync::Arc::new(tokio::sync::OnceCell::new());

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        let ota_handler = OtaHandler::start(tasks, client.clone(), &opts)
            .await
            .wrap_err("couldn't initialize ota handler")?;

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        let led_tx = {
            let (led_tx, led_rx) = mpsc::channel(EVENT_BUFFER);
            tasks.spawn(LedBlink.spawn(led_rx));
            led_tx
        };

        let (telemetry_tx, telemetry_rx) = mpsc::channel(EVENT_BUFFER);

        let telemetry = Telemetry::from_config(
            client.clone(),
            &opts.telemetry_config.unwrap_or_default(),
            opts.store_directory.clone(),
            #[cfg(feature = "containers")]
            std::sync::Arc::clone(&container_handle),
        )
        .await;

        tasks.spawn(telemetry.spawn(telemetry_rx));

        let file_transfer = Self::file_transfer(tasks);

        #[cfg(feature = "containers")]
        let containers_tx = Self::setup_containers(
            client.clone(),
            opts.containers,
            &store,
            &container_handle,
            tasks,
        )
        .await
        .wrap_err("couldn't setup the container task")?;

        #[cfg(feature = "service")]
        Self::setup_service(
            opts.service.unwrap_or_default(),
            #[cfg(feature = "containers")]
            &container_handle,
            tasks,
            cancel,
        );

        #[cfg(feature = "forwarder")]
        // Initialize the forwarder instance
        let forwarder = crate::forwarder::Forwarder::init(client.clone())
            .await
            .wrap_err("couldn't initialize the forwarder")?;

        Ok(Self {
            client,
            telemetry_tx,
            file_transfer,
            #[cfg(feature = "containers")]
            containers_tx,
            #[cfg(feature = "forwarder")]
            forwarder,
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            led_tx,
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            ota_handler,
        })
    }

    fn file_transfer(tasks: &mut JoinSet<eyre::Result<()>>) -> mpsc::Sender<FileTransferEvent> {
        let (tx, rx) = mpsc::channel(EVENT_BUFFER);

        tasks.spawn(FileTransfer::new().spawn(rx));

        tx
    }

    #[cfg(feature = "containers")]
    async fn setup_containers(
        client: C,
        config: crate::containers::ContainersConfig,
        store: &edgehog_store::db::Handle,
        container_handle: &std::sync::Arc<
            tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>,
        >,
        tasks: &mut JoinSet<eyre::Result<()>>,
    ) -> eyre::Result<mpsc::UnboundedSender<Box<edgehog_containers::requests::ContainerRequest>>>
    where
        C: Client + Clone + Send + Sync + 'static,
    {
        let (container_tx, container_rx) = mpsc::unbounded_channel();

        let containers = crate::containers::ContainerService::new(
            client,
            config,
            store,
            container_handle,
            tasks,
        )
        .await
        .wrap_err("couldn't create container service")?;

        tasks.spawn(containers.spawn_unbounded(container_rx));

        Ok(container_tx)
    }

    #[cfg(feature = "service")]
    fn setup_service(
        config: edgehog_service::config::Config,
        #[cfg(feature = "containers")] container_handle: &std::sync::Arc<
            tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>,
        >,
        tasks: &mut JoinSet<eyre::Result<()>>,
        cancel: CancellationToken,
    ) where
        C: Client + Clone + Send + Sync + 'static,
    {
        if !config.enabled {
            use tracing::debug;

            debug!("local service not enabled");

            return;
        }

        let options = match edgehog_service::service::ServiceOptions::try_from(config) {
            Ok(opt) => opt,
            Err(err) => {
                error!(error = format!("{err:#}"), "invalid service options");

                return;
            }
        };

        let service = edgehog_service::service::EdgehogService::new(
            options,
            #[cfg(feature = "containers")]
            std::sync::Arc::clone(container_handle),
        );

        tasks.spawn(async {
            info!("starting local service");

            service.run(cancel).await?;

            Ok(())
        });
    }

    #[instrument(skip(self))]
    pub async fn run(&mut self) -> eyre::Result<()>
    where
        C: Client + Send + Sync + 'static,
    {
        trace!("started running");

        #[cfg(all(feature = "systemd", target_os = "linux"))]
        crate::systemd_wrapper::systemd_notify_status("Running");

        info!("Running");

        loop {
            let event = match self.client.recv().await {
                Ok(event) => RuntimeEvent::from_event(event).wrap_err("couldn't convert event")?,
                Err(RecvError::Disconnected) => {
                    error!("the Runtime was disconnected");

                    return Ok(());
                }
                Err(err) => {
                    error!(error = %Report::new(err), "error received");

                    continue;
                }
            };

            self.handle_event(event).await;
        }
    }

    #[instrument(skip_all, fields(event = %event))]
    async fn handle_event(&mut self, event: RuntimeEvent)
    where
        C: Client + Send + Sync + 'static,
    {
        trace!("received file transfer event");

        match event {
            RuntimeEvent::Command(cmd) => {
                #[cfg(all(feature = "zbus", target_os = "linux"))]
                if cmd.is_reboot() && self.ota_handler.in_progress() {
                    error!("cannot reboot during OTA update");

                    return;
                }

                if let Err(err) = execute_command(cmd).await {
                    error!(error = %Report::new(err), "command failed to execute");
                }
            }
            RuntimeEvent::Telemetry(event) => {
                if self.telemetry_tx.send(event).await.is_err() {
                    error!("couldn't send the telemetry event");
                }
            }
            RuntimeEvent::FileTransfer(event) => {
                if self.file_transfer.send(event).await.is_err() {
                    error!("couldn't send file transfer event");
                }
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            RuntimeEvent::Led(event) => {
                if self.led_tx.send(event).await.is_err() {
                    error!("couldn't send the led event");
                }
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            RuntimeEvent::Ota(ota) => {
                if let Err(err) = self.ota_handler.handle_event(ota).await {
                    error!(
                        error = %eyre::Report::new(err),
                        "error while processing ota event",
                    );
                }
            }
            #[cfg(feature = "containers")]
            RuntimeEvent::Container(event) => {
                if self.containers_tx.send(event).is_err() {
                    error!("couldn't handle the container event")
                }
            }
            #[cfg(feature = "forwarder")]
            RuntimeEvent::Forwarder(event) => {
                self.forwarder.handle_sessions(event);
            }
        }
    }

    #[cfg(feature = "containers")]
    async fn store(
        store_dir: &std::path::Path,
    ) -> Result<edgehog_store::db::Handle, edgehog_store::db::HandleError> {
        use edgehog_store::db::Handle;

        let db_file = store_dir.join("state.db");

        let store = Handle::open(&db_file).await?;

        Ok(store)
    }
}
