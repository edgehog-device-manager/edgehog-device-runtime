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

use edgehog_config::ConfigFile;
use tokio::sync::watch;
use tracing::{error, info};

use crate::DeviceRuntimeOptions;
use crate::controller::actor::Actor;

use super::load_additional_info;
use super::loader::ConfigRepository;

/// Reloads the Configuration
pub(crate) struct ConfigReload {
    config: ConfigRepository,
    tx: watch::Sender<Option<DeviceRuntimeOptions>>,
}

impl ConfigReload {
    pub(crate) fn new(
        config: ConfigRepository,
        tx: watch::Sender<Option<DeviceRuntimeOptions>>,
    ) -> Self {
        Self { config, tx }
    }

    async fn update_config(&mut self) -> Result<(), eyre::Error> {
        let mut config = self.config.resolve_configs();

        load_additional_info(&mut config).await?;

        match DeviceRuntimeOptions::try_from(config) {
            Ok(config) => {
                self.tx.send_if_modified(|current| match current {
                    Some(current) => {
                        let modified = *current != config;

                        *current = config;

                        modified
                    }
                    None => {
                        *current = Some(config);

                        true
                    }
                });
            }
            Err(error) => {
                error!(%error, "invalid device configuration");
            }
        }

        Ok(())
    }
}

impl Actor for ConfigReload {
    type Msg = (PathBuf, Option<ConfigFile>);

    fn task() -> &'static str {
        "config-reload"
    }

    async fn init(&mut self) -> eyre::Result<()> {
        if dbg!(self.config.has_dynamic()) {
            info!("dynamic configs already stored, loading configuration");

            self.update_config().await?;
        }

        Ok(())
    }

    async fn handle(
        &mut self,
        _cancel: &tokio_util::sync::CancellationToken,
        (path, config): Self::Msg,
    ) -> eyre::Result<()> {
        match config {
            Some(config) => self.config.add_dynamic(path, config),
            None => self.config.rm_dynamic(path),
        }

        self.update_config().await?;

        Ok(())
    }
}
