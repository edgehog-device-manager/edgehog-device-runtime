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

use serde::Deserialize;

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
pub mod error;
#[cfg(feature = "file-transfer")]
pub(crate) mod file_transfer;
#[cfg(feature = "forwarder")]
mod forwarder;
pub(crate) mod http;
#[cfg(feature = "file-transfer")]
pub(crate) mod io;
#[cfg(feature = "file-transfer")]
pub(crate) mod jobs;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod led_behavior;
#[cfg(all(feature = "zbus", target_os = "linux"))]
pub mod ota;
mod power_management;
pub mod repository;
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

#[derive(Debug, Deserialize, Clone)]
pub struct DeviceManagerOptions {
    pub astarte_library: AstarteLibrary,
    pub astarte_device_sdk: Option<AstarteDeviceSdkConfigOptions>,
    #[cfg(feature = "message-hub")]
    pub astarte_message_hub: Option<data::astarte_message_hub_node::AstarteMessageHubOptions>,
    #[cfg(feature = "containers")]
    pub containers: containers::ContainersConfig,
    #[cfg(feature = "service")]
    pub service: Option<edgehog_service::config::Config>,
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    pub ota: self::ota::config::OtaConfig,
    pub interfaces_directory: PathBuf,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig<'static>>>,
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
