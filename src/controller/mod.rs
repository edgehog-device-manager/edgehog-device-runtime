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

use actor::Actor;
use astarte_device_sdk::FromEvent;
use astarte_device_sdk::client::RecvError;
use astarte_device_sdk::prelude::PropAccess;
use eyre::{Context, Report};
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, trace};

use crate::commands::execute_command;
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
    cancel: CancellationToken,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    #[cfg(feature = "file-transfer")]
    file_transfer: mpsc::Sender<crate::file_transfer::interface::request::FileTransferRequest>,
    #[cfg(feature = "containers")]
    containers_tx: mpsc::Sender<Box<edgehog_containers::requests::ContainerRequest>>,
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

        #[cfg(any(feature = "file-transfer", feature = "containers"))]
        let store = Self::store(&opts.store_directory)
            .await
            .wrap_err("couldn't connect to container store")?;

        #[cfg(feature = "file-transfer")]
        let jobs = {
            use crate::jobs::Queue;

            let jobs = Queue::new(store.clone());

            jobs.init().await?;

            jobs
        };

        #[cfg(feature = "containers")]
        let container_handle = std::sync::Arc::new(tokio::sync::OnceCell::new());

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        let ota_handler = OtaHandler::start(tasks, cancel.child_token(), client.clone(), &opts)
            .await
            .wrap_err("couldn't initialize ota handler")?;

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        let led_tx = {
            let (led_tx, led_rx) = mpsc::channel(EVENT_BUFFER);
            tasks.spawn(LedBlink.run(led_rx, cancel.child_token()));
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

        tasks.spawn(telemetry.run(telemetry_rx, cancel.child_token()));

        // TODO: Add configuration
        #[cfg(feature = "file-transfer")]
        let file_transfer = Self::file_transfer(
            client.clone(),
            tasks,
            jobs,
            opts.store_directory.join("file-store"),
            cancel.child_token(),
        )
        .wrap_err("could't initialize file transfer")?;

        #[cfg(feature = "containers")]
        let containers_tx = Self::setup_containers(
            client.clone(),
            opts.containers,
            &store,
            &container_handle,
            tasks,
            cancel.child_token(),
        )
        .await
        .wrap_err("couldn't setup the container task")?;

        #[cfg(feature = "service")]
        Self::setup_service(
            opts.service.unwrap_or_default(),
            #[cfg(feature = "containers")]
            &container_handle,
            tasks,
            cancel.child_token(),
        );

        #[cfg(feature = "forwarder")]
        // Initialize the forwarder instance
        let forwarder = crate::forwarder::Forwarder::init(client.clone())
            .await
            .wrap_err("couldn't initialize the forwarder")?;

        Ok(Self {
            client,
            cancel,
            telemetry_tx,
            #[cfg(feature = "file-transfer")]
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

    #[cfg(feature = "file-transfer")]
    fn file_transfer(
        device: C,
        tasks: &mut JoinSet<eyre::Result<()>>,
        jobs: crate::jobs::Queue,
        store_dir: std::path::PathBuf,
        cancel: CancellationToken,
    ) -> eyre::Result<mpsc::Sender<crate::file_transfer::interface::request::FileTransferRequest>>
    where
        C: Client + Send + Sync + 'static,
    {
        use std::sync::Arc;

        use tokio::sync::Notify;

        use crate::file_transfer::{self, FileTransfer, ProgressTracker};

        use self::actor::Persisted;

        let (transfer_tx, transfer_rx) = tokio::sync::mpsc::channel(EVENT_BUFFER);
        let notify = Arc::new(Notify::new());

        let (progress_tx, progress_rx) = tokio::sync::watch::channel(None);

        tasks.spawn(ProgressTracker::create(device.clone()).run(progress_rx, cancel.clone()));
        tasks.spawn(
            FileTransfer::create(jobs.clone(), store_dir, device.clone(), progress_tx)?
                .run(Arc::clone(&notify), cancel.clone()),
        );
        tasks.spawn(file_transfer::Receiver::new(jobs, notify, device).run(transfer_rx, cancel));

        Ok(transfer_tx)
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
        cancel: CancellationToken,
    ) -> eyre::Result<mpsc::Sender<Box<edgehog_containers::requests::ContainerRequest>>>
    where
        C: Client + Send + Sync + 'static,
    {
        let (container_tx, container_rx) = mpsc::channel(EVENT_BUFFER);

        let containers = crate::containers::ContainerService::new(
            client,
            config,
            store,
            container_handle,
            tasks,
        )
        .await
        .wrap_err("couldn't create container service")?;

        tasks.spawn(containers.run(container_rx, cancel));

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

        while let Some(res) = self.cancel.run_until_cancelled(self.client.recv()).await {
            let event = match res {
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

        Ok(())
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
            #[cfg(feature = "file-transfer")]
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
                if self.containers_tx.send(event).await.is_err() {
                    error!("couldn't handle the container event")
                }
            }
            #[cfg(feature = "forwarder")]
            RuntimeEvent::Forwarder(event) => {
                self.forwarder.handle_sessions(event);
            }
        }
    }

    #[cfg(any(feature = "file-transfer", feature = "containers"))]
    async fn store(
        store_dir: &std::path::Path,
    ) -> Result<edgehog_store::db::Handle, edgehog_store::db::HandleError> {
        let db_file = store_dir.join("state.db");

        let store = edgehog_store::db::Handle::open(&db_file).await?;

        Ok(store)
    }
}
