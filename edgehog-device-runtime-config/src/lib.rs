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

//! Configuration File for the Edgehog Device Runtime.

#![warn(
    missing_docs,
    rustdoc::missing_crate_level_docs,
    clippy::dbg_macro,
    clippy::todo
)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use self::container::ContainersConfig;
use self::file_transfer::FileTransferConfig;
use self::forwarder::ForwarderConfig;
use self::ota::OtaConfig;
use self::sdk::{AstarteLibrary, DeviceSdkConfig, MsgHubConfig};
use self::service::ServiceConfig;
use self::telemetry::TelemetryInterfaceConfig;

pub mod container;
pub mod file_transfer;
pub mod forwarder;
pub mod ota;
pub mod sdk;
pub mod service;
pub mod telemetry;

/// Configuration file
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Connection type to use to connect with Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_library: Option<AstarteLibrary>,
    /// Astarte device sdk options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_device_sdk: Option<DeviceSdkConfig>,
    /// Astarte message hub options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_message_hub: Option<MsgHubConfig>,
    /// Container service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containers: Option<ContainersConfig>,
    /// Local Edgehog service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceConfig>,
    /// Ota configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ota: Option<OtaConfig>,
    /// File transfer service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_transfer: Option<FileTransferConfig>,
    /// Edgehog device forwarder service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarder: Option<ForwarderConfig>,
    /// Edgehog interface directory
    #[deprecated(note = "the interfaces are included in the binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interfaces_directory: Option<PathBuf>,
    /// Persistent store directory writable by Edgehog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_directory: Option<PathBuf>,
    /// Download directory for the OTA updates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<PathBuf>,
    /// Configuration for the telemetries to send to Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig>>,
}
