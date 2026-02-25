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

//! Telemetry sent by the device periodically that can be configured.

use std::fmt::Display;
use std::str::FromStr;

use astarte_device_sdk::Client;

use self::storage_usage::StorageUsage;
use self::system_status::SystemStatusTelemetry;

use super::sender::TelemetryTask;

#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod battery_status;
#[cfg(feature = "containers")]
pub(crate) mod container;
pub(crate) mod storage_usage;
pub(crate) mod system_status;
#[cfg(feature = "wifiscanner")]
pub(crate) mod wifi_scan;

#[derive(Debug, thiserror::Error)]
#[error("unsupported telemetry interface: {interface}")]
pub struct TelemetryInterfaceError {
    interface: String,
}

/// Sends the initial telemetry on startup
pub(crate) async fn initial_telemetry<C>(client: &mut C)
where
    C: Client + Send + Sync + 'static,
{
    // Those are telemetry sent periodically
    SystemStatusTelemetry::default().send(client).await;

    StorageUsage::default().send(client).await;

    #[cfg(feature = "wifiscanner")]
    self::wifi_scan::WifiScan::default().send(client).await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum TelemetryInterface {
    SystemStatus,
    StorageUsage,
    BatteryStatus,
    WiFiScanResults,
    ContainerBlkio,
    ContainerCpu,
    ContainerMemory,
    ContainerMemoryStats,
    ContainerNetworks,
    ContainerProcesses,
    VolumeUsage,
}

impl TelemetryInterface {
    pub const fn as_interface(&self) -> &'static str {
        match self {
            TelemetryInterface::SystemStatus => system_status::INTERFACE,
            TelemetryInterface::StorageUsage => storage_usage::INTERFACE,
            TelemetryInterface::BatteryStatus => "io.edgehog.devicemanager.BatteryStatus",
            TelemetryInterface::WiFiScanResults => "io.edgehog.devicemanager.WiFiScanResults",
            TelemetryInterface::ContainerBlkio => {
                "io.edgehog.devicemanager.apps.stats.ContainerBlkio"
            }
            TelemetryInterface::ContainerCpu => "io.edgehog.devicemanager.apps.stats.ContainerCpu",
            TelemetryInterface::ContainerMemory => {
                "io.edgehog.devicemanager.apps.stats.ContainerMemory"
            }
            TelemetryInterface::ContainerMemoryStats => {
                "io.edgehog.devicemanager.apps.stats.ContainerMemoryStats"
            }
            TelemetryInterface::ContainerNetworks => {
                "io.edgehog.devicemanager.apps.stats.ContainerNetworks"
            }
            TelemetryInterface::ContainerProcesses => {
                "io.edgehog.devicemanager.apps.stats.ContainerProcesses"
            }
            TelemetryInterface::VolumeUsage => "io.edgehog.devicemanager.apps.stats.VolumeUsage",
        }
    }
}

impl FromStr for TelemetryInterface {
    type Err = TelemetryInterfaceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let telemetry = match s {
            "io.edgehog.devicemanager.SystemStatus" => TelemetryInterface::SystemStatus,
            "io.edgehog.devicemanager.StorageUsage" => TelemetryInterface::StorageUsage,
            "io.edgehog.devicemanager.BatteryStatus" => TelemetryInterface::BatteryStatus,
            "io.edgehog.devicemanager.WiFiScanResults" => TelemetryInterface::WiFiScanResults,
            "io.edgehog.devicemanager.apps.stats.ContainerBlkio" => {
                TelemetryInterface::ContainerBlkio
            }
            "io.edgehog.devicemanager.apps.stats.ContainerCpu" => TelemetryInterface::ContainerCpu,
            "io.edgehog.devicemanager.apps.stats.ContainerMemory" => {
                TelemetryInterface::ContainerMemory
            }
            "io.edgehog.devicemanager.apps.stats.ContainerMemoryStats" => {
                TelemetryInterface::ContainerMemoryStats
            }
            "io.edgehog.devicemanager.apps.stats.ContainerNetworks" => {
                TelemetryInterface::ContainerNetworks
            }
            "io.edgehog.devicemanager.apps.stats.ContainerProcesses" => {
                TelemetryInterface::ContainerProcesses
            }
            "io.edgehog.devicemanager.apps.stats.VolumeUsage" => TelemetryInterface::VolumeUsage,
            _ => {
                return Err(TelemetryInterfaceError {
                    interface: s.to_string(),
                });
            }
        };

        Ok(telemetry)
    }
}

impl Display for TelemetryInterface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_interface())
    }
}

#[derive(Debug)]
pub(crate) enum ContainerInterface {
    ContainerBlkio,
    ContainerCpu,
    ContainerMemory,
    ContainerMemoryStats,
    ContainerNetworks,
    ContainerProcesses,
    VolumeUsage,
}

impl Display for ContainerInterface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            ContainerInterface::ContainerBlkio => "container blkio",
            ContainerInterface::ContainerCpu => "container cpu",
            ContainerInterface::ContainerMemory => "container memory",
            ContainerInterface::ContainerMemoryStats => "container memory stats",
            ContainerInterface::ContainerNetworks => "container networks",
            ContainerInterface::ContainerProcesses => "container processes",
            ContainerInterface::VolumeUsage => "volume usage",
        };

        write!(f, "{str}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_parse_interface() {
        let valid = [
            (
                "io.edgehog.devicemanager.SystemStatus",
                TelemetryInterface::SystemStatus,
            ),
            (
                "io.edgehog.devicemanager.StorageUsage",
                TelemetryInterface::StorageUsage,
            ),
            (
                "io.edgehog.devicemanager.BatteryStatus",
                TelemetryInterface::BatteryStatus,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerBlkio",
                TelemetryInterface::ContainerBlkio,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerCpu",
                TelemetryInterface::ContainerCpu,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerMemory",
                TelemetryInterface::ContainerMemory,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerMemoryStats",
                TelemetryInterface::ContainerMemoryStats,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerNetworks",
                TelemetryInterface::ContainerNetworks,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.ContainerProcesses",
                TelemetryInterface::ContainerProcesses,
            ),
            (
                "io.edgehog.devicemanager.apps.stats.VolumeUsage",
                TelemetryInterface::VolumeUsage,
            ),
        ];

        for (i, exp) in valid {
            assert_eq!(TelemetryInterface::from_str(i).unwrap(), exp);
        }

        assert!(TelemetryInterface::from_str("foo").is_err());
    }
}
