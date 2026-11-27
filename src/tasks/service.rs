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

use std::path::PathBuf;
#[cfg(feature = "containers")]
use std::sync::Arc;

use edgehog_service::config::ServiceArgs;
use edgehog_service::service::config::ConfigState;
use edgehog_service::service::{EdgehogService, ServiceOptions};
use eyre::{Context, OptionExt};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio_util::either::Either;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::DeviceRuntimeOptions;
use crate::config::reload::ConfigReload;
use crate::controller::EVENT_BUFFER;
use crate::controller::actor::Actor;
use crate::tasks::{handle_join, spawn_runtime};

use super::{Tasks, join_tasks};

impl Tasks {
    /// Set up the local service and enables the configuration reload.
    pub(crate) async fn service_setup(
        &mut self,
        dynamic_configs: PathBuf,
        service_args: ServiceArgs,
    ) -> eyre::Result<()> {
        #[cfg(feature = "containers")]
        let container_handle = Arc::default();

        let options = ServiceOptions::try_from(service_args)?;
        let (tx, rx) = tokio::sync::mpsc::channel(EVENT_BUFFER);
        let config = ConfigState::new(dynamic_configs, tx);

        let service = EdgehogService::new(options, config);

        #[cfg(feature = "containers")]
        let mut service = service;
        #[cfg(feature = "containers")]
        service.set_container_handle(container_handle);

        self.tasks.spawn({
            let cancel = self.cancel.child_token();

            async {
                info!("starting local service");

                service.run(cancel).await?;

                Ok(())
            }
        });

        let (config_tx, config_rx) = tokio::sync::watch::channel(None);

        let reload = ConfigReload::new(self.config.as_ref().clone(), config_tx);

        self.tasks.spawn(reload.run(rx, self.cancel.child_token()));

        self.tasks.spawn({
            let cancel = self.cancel.child_token();

            async move { runtime_with_config(config_rx, cancel).await }
        });

        Ok(())
    }
}

// TODO We should only reconfigure the tasks and not require a complete reconfiguration of all the fields
async fn runtime_with_config(
    mut config_rx: watch::Receiver<Option<DeviceRuntimeOptions>>,
    parent_token: CancellationToken,
) -> eyre::Result<()> {
    // Token used to restart the tasks.
    let mut child_token = parent_token.child_token();

    // Additional  to
    let mut tasks = JoinSet::new();

    let Some(opt_res) = parent_token
        .run_until_cancelled(config_rx.wait_for(|opt| opt.is_some()))
        .await
    else {
        info!("runtime tasks exited");

        return Ok(());
    };

    let options = opt_res
        .wrap_err("couldn't receive configuration")
        .and_then(|opt| opt.clone().ok_or_eyre("configuration still missing"))?;

    spawn_runtime(options, &mut tasks, child_token.child_token()).await?;

    'tasks: loop {
        match select(&mut config_rx, &mut tasks).await? {
            Either::Left(options) => {
                handle_new_config(options, &mut tasks, &parent_token, &mut child_token).await?;
            }
            Either::Right(opt_res) => {
                let Some(res) = opt_res else {
                    break 'tasks;
                };

                handle_join(&mut tasks, &parent_token, res).await?;
            }
        }
    }

    info!("runtime tasks exited");

    Ok(())
}

async fn select(
    config_rx: &mut watch::Receiver<Option<DeviceRuntimeOptions>>,
    tasks: &mut JoinSet<eyre::Result<()>>,
) -> eyre::Result<
    Either<DeviceRuntimeOptions, Option<Result<eyre::Result<()>, tokio::task::JoinError>>>,
> {
    tokio::select! {
        opt_res = config_rx.wait_for(|opt| opt.is_some()) => {
            let options = opt_res
                .wrap_err("couldn't receive configuration")
                .and_then(|opt| opt.clone().ok_or_eyre("configuration still missing"))?;


            Ok(Either::Left(options))
        }
        opt_res = tasks.join_next() => {
            Ok(Either::Right(opt_res))
        }
    }
}

async fn handle_new_config(
    options: DeviceRuntimeOptions,
    tasks: &mut JoinSet<eyre::Result<()>>,
    parent_token: &CancellationToken,
    child_token: &mut CancellationToken,
) -> eyre::Result<()> {
    child_token.cancel();
    // replace the child token
    *child_token = parent_token.child_token();

    join_tasks(tasks, parent_token).await?;

    spawn_runtime(options, tasks, child_token.child_token()).await?;

    Ok(())
}
