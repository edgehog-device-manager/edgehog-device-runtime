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

use eyre::Context;
use serde::Deserialize;
use tracing::info;

pub use self::controller::Runtime;
use self::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
use self::telemetry::TelemetryInterfaceConfig;
pub use astarte_device_sdk::Client;

mod commands;
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
pub enum AstarteLibrary {
    #[serde(rename = "astarte-device-sdk")]
    AstarteDeviceSdk,
    #[cfg(feature = "message-hub")]
    #[serde(rename = "astarte-message-hub")]
    AstarteMessageHub,
}

#[derive(Debug, Clone)]
pub struct DeviceManagerOptions {
    pub astarte_library: AstarteLibrary,
    pub astarte_device_sdk: Option<AstarteDeviceSdkConfigOptions>,
    #[cfg(feature = "message-hub")]
    pub astarte_message_hub: Option<data::astarte_message_hub_node::AstarteMessageHubOptions>,
    #[cfg(feature = "containers")]
    pub containers: containers::ContainersConfig,
    #[cfg(feature = "service")]
    pub service: Option<edgehog_service::config::Config>,
    #[cfg(feature = "file-transfer")]
    pub file_transfer: self::file_transfer::config::FileTransferArgs,
    #[cfg(feature = "forwarder")]
    pub forwarder: self::forwarder::ForwarderConfig,
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    pub ota: self::ota::config::OtaConfig,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig<'static>>>,
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
