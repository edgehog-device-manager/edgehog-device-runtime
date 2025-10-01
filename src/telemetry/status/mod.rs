// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

//! Initial telemetry sent by the device on startup.

use astarte_device_sdk::Client;
use tracing::debug;

#[cfg(all(feature = "zbus", target_os = "linux"))]
use self::cellular_properties::CellularConnection;
use self::hardware_info::HardwareInfo;
use self::os_release::OsRelease;
use self::runtime_info::RUNTIME_INFO;
use self::system_info::SystemInfo;

#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod cellular_properties;
pub mod hardware_info;
#[cfg(feature = "udev")]
pub(crate) mod net_interfaces;
pub mod os_release;
pub mod runtime_info;
pub(crate) mod system_info;

/// Sends the initial telemetry on startup
pub(crate) async fn initial_telemetry<C>(client: &mut C)
where
    C: Client,
{
    #[cfg(feature = "systemd")]
    crate::systemd_wrapper::systemd_notify_status("Sending initial telemetry");

    if let Some(os_release) = OsRelease::read().await {
        debug!("couldn't read os release information");

        os_release.send(client).await;
    }

    HardwareInfo::read().await.send(client).await;

    RUNTIME_INFO.send(client).await;

    #[cfg(feature = "udev")]
    self::net_interfaces::send_network_interface_properties(client).await;

    SystemInfo::read().send(client).await;

    #[cfg(all(feature = "zbus", target_os = "linux"))]
    CellularConnection::read().await.send(client).await;
}
