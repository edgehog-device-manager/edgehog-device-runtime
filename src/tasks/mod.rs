// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Manages the spawning and joining of Tokio tasks

use std::sync::Arc;
use std::time::Duration;

use astarte_device_sdk::client::ClientConnection;
use eyre::{Context, OptionExt, eyre};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, trace};

use crate::cli::Cli;
use crate::config::loader::ConfigRepository;
use crate::config::{load_additional_info, read_options};
use crate::data::connect_store;
use crate::{AstarteOptions, DeviceRuntimeOptions, Runtime};

#[cfg(feature = "service")]
mod service;

pub const TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Tasks {
    config: Arc<ConfigRepository>,
    tasks: JoinSet<eyre::Result<()>>,
    cancel: CancellationToken,
}

impl Tasks {
    /// Create a task set for the runtime
    pub async fn init(cli: Cli) -> eyre::Result<Self> {
        let loader = read_options(cli).await?;

        let tasks = JoinSet::new();
        let cancel = CancellationToken::new();

        Ok(Self {
            config: Arc::new(loader),
            tasks,
            cancel,
        })
    }

    /// Spawns all the runtime tasks
    pub async fn spawn(&mut self) -> eyre::Result<()> {
        let mut config = self.config.resolve_configs();

        self.shutdown();

        #[cfg(feature = "service")]
        {
            let service_args = edgehog_service::config::ServiceArgs::from(config.service.clone());

            if service_args.enabled {
                // TODO: resolve the config directory
                let dynamic_configs = config
                    .store_directory
                    .ok_or_eyre("missing storage directory")?
                    .join("config");

                self.service_setup(dynamic_configs, service_args).await?;

                return Ok(());
            } else {
                debug!("local service disabled");
            }
        }

        load_additional_info(&mut config).await?;

        let options = DeviceRuntimeOptions::try_from(config)?;

        spawn_runtime(options, &mut self.tasks, self.cancel.child_token()).await
    }

    /// Waits for all of the runtime tasks to exit
    pub async fn join(&mut self) -> eyre::Result<()> {
        join_tasks(&mut self.tasks, &self.cancel).await
    }

    #[instrument(skip_all)]
    fn shutdown(&mut self) {
        let cancel = self.cancel.clone();

        self.tasks.spawn(async move {
            let shutdown = async {
                #[cfg(unix)]
                if cfg!(unix) {
                    use eyre::Context;
                    use tokio::signal::unix::SignalKind;
                    use tracing::info;

                    let mut term = tokio::signal::unix::signal(SignalKind::terminate())
                        .wrap_err("couldn't create SIGTERM listener")?;
                    let mut int = tokio::signal::unix::signal(SignalKind::interrupt())
                        .wrap_err("couldn't create SIGINT listener")?;

                    tokio::select! {
                        _ = term.recv() => {
                            info!("SIGTERM received");
                        }
                        _ = int.recv() => {
                            info!("SIGINT received");
                        }
                    }

                    return Ok::<(), eyre::Report>(());
                }

                tokio::signal::ctrl_c()
                    .await
                    .wrap_err("couldn't wait for CTRL-C signal")?;

                Ok::<(), eyre::Report>(())
            };

            // Exit if other tasks error
            match cancel.run_until_cancelled(shutdown).await {
                Some(res) => {
                    cancel.cancel();

                    res
                }
                None => {
                    debug!("canceled");

                    Ok(())
                }
            }
        });
    }
}

async fn spawn_runtime(
    options: DeviceRuntimeOptions,
    tasks: &mut JoinSet<eyre::Result>,
    cancel: CancellationToken,
) -> eyre::Result<()> {
    let store = connect_store(&options.store_directory).await?;

    match &options.astarte_library {
        AstarteOptions::Sdk(device_options) => {
            let mut client = device_options.connect(tasks, store, &options).await?;

            let mut runtime = Runtime::new(tasks, options, client.clone(), cancel.clone()).await?;

            tasks.spawn(async move {
                runtime
                    .run()
                    .await
                    .wrap_err("the Device Runtime encountered an unrecoverable error")
            });

            tasks.spawn({
                let cancel = cancel.child_token();
                async move {
                    cancel.cancelled().await;

                    info!("disconnecting client");

                    client.disconnect().await?;

                    Ok(())
                }
            });
        }
        #[cfg(feature = "message-hub")]
        AstarteOptions::MessageHub(msghub_options) => {
            let mut client = msghub_options.connect(tasks, store, &options).await?;

            let mut runtime = Runtime::new(tasks, options, client.clone(), cancel.clone()).await?;

            tasks.spawn(async move {
                runtime
                    .run()
                    .await
                    .wrap_err("the Device Runtime encountered an unrecoverable error")
            });

            tasks.spawn({
                let cancel = cancel.child_token();

                async move {
                    cancel.cancelled().await;

                    info!("disconnecting client");

                    client.disconnect().await?;

                    Ok(())
                }
            });
        }
    };

    Ok(())
}

/// Waits for all of the runtime tasks to exit
pub(crate) async fn join_tasks(
    tasks: &mut JoinSet<eyre::Result>,
    cancel: &CancellationToken,
) -> eyre::Result<()> {
    while let Some(res) = tasks.join_next().await {
        handle_join(tasks, cancel, res).await?;
    }

    Ok(())
}

pub(crate) async fn handle_join(
    tasks: &mut JoinSet<Result<(), eyre::Error>>,
    cancel: &CancellationToken,
    res: Result<Result<(), eyre::Error>, tokio::task::JoinError>,
) -> Result<(), eyre::Error> {
    match res {
        Ok(Ok(())) => {
            trace!("task joined");

            Ok(())
        }
        Ok(Err(err)) => {
            error!(error = format!("{err:#}"), "task joined");

            cancel.cancel();

            wait_exit(tasks).await?;

            Err(err)
        }
        Err(err) if err.is_cancelled() => {
            debug!(error = %err, "task joined");

            Ok(())
        }
        Err(err) => {
            error!(error = %err, "task joined");

            cancel.cancel();

            wait_exit(tasks).await?;

            Err(err).wrap_err("task failed")
        }
    }
}

async fn wait_exit(tasks: &mut JoinSet<eyre::Result>) -> eyre::Result<()> {
    info!(
        timeout_secs = TASK_SHUTDOWN_TIMEOUT.as_secs(),
        "waiting for tasks to join cleanly ",
    );

    // join all tasks with a global timeout
    let deadline = tokio::time::Instant::now()
        .checked_add(TASK_SHUTDOWN_TIMEOUT)
        .ok_or_eyre("incorrect instant now")?;

    let mut task_errors = false;

    while let Some(join_res) = tokio::time::timeout_at(deadline, tasks.join_next())
        .await
        .wrap_err("timeout reached while waiting for tasks to shutdown cleanly")?
    {
        match join_res {
            Ok(Ok(())) => {
                debug!("task joined");
            }
            Ok(Err(error)) => {
                error!(%error, "task exited with an error");

                task_errors = true;
            }
            Err(error) => {
                error!(%error, "couldn't join a task");

                task_errors = true;
            }
        }
    }

    if !task_errors {
        Ok(())
    } else {
        Err(eyre!("one or more task failed"))
    }
}
