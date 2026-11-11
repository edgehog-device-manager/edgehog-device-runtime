// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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
use edgehog_config::sdk::AstarteLibrary;
use eyre::{Context, OptionExt};
use serde::Deserialize;
use tracing::{info, trace, warn};

pub use self::controller::Runtime;
use self::data::device_sdk::DeviceSdkOptions;
#[cfg(feature = "message-hub")]
use self::data::message_hub::MessageHubOptions;
use self::telemetry::TelemetryInterfaceArgs;
pub use astarte_device_sdk::Client;

pub mod cli;
mod commands;
pub mod config;
#[cfg(feature = "containers")]
pub mod containers;
mod controller;
pub mod data;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod device;
#[cfg(feature = "file-transfer")]
pub mod file_transfer;
#[cfg(feature = "forwarder")]
pub mod forwarder;
pub(crate) mod http;
#[cfg(feature = "file-transfer")]
pub mod io;
#[cfg(feature = "file-transfer")]
pub(crate) mod jobs;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod led_behavior;
#[cfg(all(feature = "zbus", target_os = "linux"))]
pub mod ota;
mod power_management;
pub mod repository;
#[cfg(feature = "file-transfer")]
pub mod storage;
#[cfg(all(feature = "systemd", target_os = "linux"))]
pub mod systemd_wrapper;
pub mod telemetry;

#[derive(Deserialize, Debug, Clone)]
pub enum AstarteOptions {
    Sdk(DeviceSdkOptions),
    #[cfg(feature = "message-hub")]
    MessageHub(MessageHubOptions),
}

#[derive(Debug, Clone)]
pub struct DeviceManagerOptions {
    pub astarte_library: AstarteOptions,
    #[cfg(feature = "containers")]
    pub containers: containers::ContainersArgs,
    #[cfg(feature = "service")]
    pub service: edgehog_service::config::ServiceArgs,
    #[cfg(feature = "file-transfer")]
    pub file_transfer: self::file_transfer::config::FileTransferArgs,
    #[cfg(feature = "forwarder")]
    pub forwarder: self::forwarder::ForwarderArgs,
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    pub ota: self::ota::config::OtaArgs,
    pub telemetry_config: Vec<TelemetryInterfaceArgs<'static>>,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
}

impl DeviceManagerOptions {
    pub async fn setup_dirs(mut self) -> eyre::Result<Self> {
        tokio::fs::create_dir_all(&self.download_directory)
            .await
            .wrap_err("unable to create OTA download directory.")?;
        let download = tokio::fs::canonicalize(&self.download_directory).await?;

        tokio::fs::create_dir_all(&self.store_directory)
            .await
            .wrap_err("unable to create store directory")?;
        let store = tokio::fs::canonicalize(&self.store_directory).await?;

        info!(store = %store.display(), download = %download.display(), "using store directories");

        #[cfg(feature = "file-transfer")]
        {
            self.file_transfer.storage_dir = {
                let ft_dir = &self.file_transfer.storage_dir;

                tokio::fs::create_dir_all(ft_dir).await?;
                let ft_dir = tokio::fs::canonicalize(ft_dir).await?;

                info!(file_transfer = %ft_dir.display(), "using file transfer directory");

                ft_dir
            };
        }

        self.store_directory = store;
        self.download_directory = download;

        Ok(self)
    }
}

impl TryFrom<ConfigFile> for DeviceManagerOptions {
    type Error = eyre::Error;

    fn try_from(value: ConfigFile) -> Result<Self, Self::Error> {
        let ConfigFile {
            astarte_library,
            astarte_device_sdk,
            astarte_message_hub,
            containers,
            service,
            ota,
            file_transfer,
            forwarder,
            #[expect(deprecated)]
            interfaces_directory,
            store_directory,
            download_directory,
            telemetry_config,
        } = value;

        let astarte_library = match astarte_library.unwrap_or_default() {
            AstarteLibrary::AstarteDeviceSdk => {
                trace!("configured the astarte sdk library");

                let config =
                    astarte_device_sdk.ok_or_eyre("missing Astarte device SDK configuration")?;

                let options = DeviceSdkOptions::try_from(config)?;

                AstarteOptions::Sdk(options)
            }
            #[cfg(feature = "message-hub")]
            AstarteLibrary::AstarteMessageHub => {
                trace!("configured the messge hub");

                let config =
                    astarte_message_hub.ok_or_eyre("missing Astarte device SDK configuration")?;

                let options = MessageHubOptions::try_from(config)?;

                AstarteOptions::MessageHub(options)
            }
        };

        let store_directory = store_directory.ok_or_eyre("config is missing store directory")?;

        let download_directory =
            download_directory.unwrap_or_else(|| store_directory.join("download"));

        #[cfg(feature = "file-transfer")]
        let file_transfer = self::file_transfer::config::FileTransferArgs::with_store_dir(
            file_transfer,
            &store_directory,
        );

        if interfaces_directory.is_some() {
            warn!(
                "DEPRECATED: the interface directory is no longer used, the interfaces are included in the binary"
            )
        }

        let telemetry_config = telemetry_config
            .unwrap_or_default()
            .into_iter()
            .map(TelemetryInterfaceArgs::from)
            .collect();

        Ok(Self {
            astarte_library,
            #[cfg(feature = "containers")]
            containers: self::containers::ContainersArgs::from(containers),
            #[cfg(feature = "service")]
            service: edgehog_service::config::ServiceArgs::from(service),
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            ota: ota::config::OtaArgs::from(ota),
            #[cfg(feature = "file-transfer")]
            file_transfer,
            #[cfg(feature = "forwarder")]
            forwarder: self::forwarder::ForwarderArgs::from(forwarder),
            telemetry_config,
            store_directory,
            download_directory,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use insta::assert_snapshot;

    #[cfg(feature = "file-transfer")]
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Hexdump<T>(pub(crate) T)
    where
        T: std::borrow::Borrow<[u8]>;

    #[cfg(feature = "file-transfer")]
    impl<T> std::fmt::Display for Hexdump<T>
    where
        T: std::borrow::Borrow<[u8]>,
    {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let buf = self.0.borrow();
            writeln!(f, "Length: {} ({:#x}) bytes", buf.len(), buf.len())?;

            for (i, b) in buf.iter().map(|b| b.to_le()).enumerate() {
                let b_h = b >> 4;
                let b_l = b & 0x0f;

                let c = if b.is_ascii_graphic() { b as char } else { '.' };

                writeln!(f, "{i:04}: | {b_h:04b} {b_l:04b} | ({b:#04x}) '{c}'")?;
            }

            Ok(())
        }
    }

    macro_rules! with_insta {
        ($asserts:block) => {
            ::insta::with_settings!({
                snapshot_path => concat!(env!("CARGO_MANIFEST_DIR"), "/snapshots")
            }, $asserts);
        };
    }

    pub(crate) use with_insta;

    #[test]
    fn use_macro() {
        self::with_insta!({
            assert_snapshot!("using the macro");
        });
    }
}
