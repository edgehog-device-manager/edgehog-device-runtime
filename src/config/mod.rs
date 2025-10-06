// This file is part of Edgehog.
//
// Copyright 2022 - 2025 SECO Mind Srl
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

use std::path::{Path, PathBuf};

use edgehog_device_runtime::telemetry::TelemetryInterfaceConfig;
use edgehog_device_runtime::{AstarteLibrary, DeviceManagerOptions};
use serde::Deserialize;
use stable_eyre::eyre::{ensure, OptionExt};
use tracing::info;

use crate::cli::{Cli, Command, DeviceSdkArgs, OverrideOption};

/// Configuration file
#[derive(Debug, Deserialize, Clone, Default)]
pub struct Config {
    pub astarte_library: Option<AstarteLibrary>,

    pub astarte_device_sdk: Option<DeviceSdkArgs>,
    #[cfg(feature = "message-hub")]
    pub astarte_message_hub: Option<crate::cli::MsgHubArgs>,

    #[cfg(feature = "containers")]
    pub containers: Option<edgehog_device_runtime::containers::ContainersConfig>,

    #[cfg(feature = "service")]
    pub service: Option<edgehog_service::config::Config>,

    #[cfg(all(feature = "zbus", target_os = "linux"))]
    pub ota: Option<edgehog_device_runtime::ota::config::OtaConfig>,

    pub interfaces_directory: Option<PathBuf>,
    pub store_directory: Option<PathBuf>,
    pub download_directory: Option<PathBuf>,
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig<'static>>>,
}

impl TryFrom<Config> for DeviceManagerOptions {
    type Error = stable_eyre::eyre::Error;

    fn try_from(value: Config) -> Result<Self, Self::Error> {
        let astarte_library = value
            .astarte_library
            .ok_or_eyre("config is missing astarte_library value")?;

        let astarte_device_sdk = value
            .astarte_device_sdk
            .map(|opt| opt.try_into())
            .transpose()?;

        #[cfg(feature = "message-hub")]
        let astarte_message_hub = value
            .astarte_message_hub
            .map(|opt| opt.try_into())
            .transpose()?;

        let interfaces_directory = value
            .interfaces_directory
            .ok_or_eyre("config is missing interfaces directory")?;

        let store_directory = value
            .store_directory
            .ok_or_eyre("config is missing store directory")?;

        let download_directory = value
            .download_directory
            .unwrap_or(store_directory.join("download"));

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        let ota = value.ota.unwrap_or_default();

        Ok(Self {
            astarte_library,
            astarte_device_sdk,
            #[cfg(feature = "message-hub")]
            astarte_message_hub,
            #[cfg(feature = "containers")]
            containers: value.containers.unwrap_or_default(),
            #[cfg(feature = "service")]
            service: value.service,
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            ota,
            interfaces_directory,
            store_directory,
            download_directory,
            telemetry_config: value.telemetry_config,
        })
    }
}

pub async fn read_options(cli: Cli) -> stable_eyre::Result<DeviceManagerOptions> {
    let paths = [
        Path::new("edgehog-config.toml"),
        Path::new("/etc/edgehog/config.toml"),
    ];

    if let Some(path) = &cli.config {
        ensure!(
            path.exists(),
            "configuration file {} doesn't exists",
            path.display()
        );
    }

    let override_config_file_path = cli.config.or(cli.configuration_file);

    let mut paths = override_config_file_path
        .as_deref()
        .into_iter()
        .chain(paths)
        .filter(|f| f.is_file());

    let mut config = if let Some(path) = paths.next() {
        info!(config = %path.display(), "reading config file");

        let config = tokio::fs::read_to_string(path).await?;

        toml::from_str(&config)?
    } else {
        Config::default()
    };

    match cli.command {
        Some(Command::DeviceSdk { device, shared }) => {
            config.astarte_library = Some(AstarteLibrary::AstarteDeviceSdk);

            if let Some(sdk) = &mut config.astarte_device_sdk {
                sdk.merge(device)
            } else {
                config.astarte_device_sdk = Some(device);
            }

            config.interfaces_directory.merge(shared.interfaces_dir);
            config.store_directory.merge(shared.store_dir);
        }
        None => {
            // Default to the sdk if it's not set, since the msg-hub config could be set in the file
            config.astarte_library = config
                .astarte_library
                .or(Some(AstarteLibrary::AstarteDeviceSdk));

            if let Some(device) = cli.device {
                if let Some(sdk) = &mut config.astarte_device_sdk {
                    sdk.merge(device)
                } else {
                    config.astarte_device_sdk = Some(device);
                }
            }

            if let Some(shared) = cli.shared {
                config.interfaces_directory.merge(shared.interfaces_dir);
                config.store_directory.merge(shared.store_dir);
            }
        }
        #[cfg(feature = "message-hub")]
        Some(Command::MsgHub { msghub, shared }) => {
            if let Some(hub) = &mut config.astarte_message_hub {
                hub.merge(msghub);
            } else {
                config.astarte_message_hub = Some(msghub);
            }

            config.interfaces_directory.merge(shared.interfaces_dir);
            config.store_directory.merge(shared.store_dir);
        }
    }

    config.try_into()
}
