// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Task to send the data.

use std::{fmt::Display, str::FromStr, time::Duration};

use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use super::{storage_usage::StorageUsage, system_status::SystemStatus};
use crate::Client;

#[derive(Debug, thiserror::Error)]
#[error("unsupported telemetry interface: {interface}")]
pub struct TelemetryInterfaceError {
    interface: String,
}

#[derive(Debug)]
pub struct Task<T> {
    client: T,
    interface: TelemetryInterface,
    cancel: CancellationToken,
    period: Duration,
}

impl<C> Task<C> {
    pub fn new(
        client: C,
        interface: TelemetryInterface,
        cancel: CancellationToken,
        period: Duration,
    ) -> Self {
        Self {
            client,
            interface,
            cancel,
            period,
        }
    }

    pub async fn run(mut self)
    where
        C: Client,
    {
        let mut interval = tokio::time::interval(self.period);

        while self
            .cancel
            .run_until_cancelled(interval.tick())
            .await
            .is_some()
        {
            info!(interface = %self.interface, "collecting telemetry",);

            self.send().await;
        }

        debug!(interface = %self.interface, "telemetry task cancelled");
    }

    async fn send(&mut self)
    where
        C: Client,
    {
        match self.interface {
            TelemetryInterface::SystemStatus => {
                let Some(sys_status) = SystemStatus::read() else {
                    return;
                };

                sys_status.send(&mut self.client).await;
            }
            TelemetryInterface::StorageUsage => {
                StorageUsage::read().send(&mut self.client).await;
            }
            TelemetryInterface::BatteryStatus => {
                cfg_if::cfg_if! {
                    if #[cfg(all(feature = "zbus", target_os = "linux"))] {
                        super::battery_status::send_battery_status(&mut self.client).await;
                    } else {
                        tracing::warn!("The battery status telemetry interface is not supported because the zbus feature is missing")
                    }
                }
            }
            TelemetryInterface::ContainerBlkio => todo!(),
            TelemetryInterface::ContainerCpu => todo!(),
            TelemetryInterface::ContainerMemory => todo!(),
            TelemetryInterface::ContainerMemoryStats => todo!(),
            TelemetryInterface::ContainerNetworks => todo!(),
            TelemetryInterface::ContainerProcesses => todo!(),
            TelemetryInterface::VolumeUsage => todo!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TelemetryInterface {
    SystemStatus,
    StorageUsage,
    BatteryStatus,
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
            TelemetryInterface::SystemStatus => "io.edgehog.devicemanager.SystemStatus",
            TelemetryInterface::StorageUsage => "io.edgehog.devicemanager.StorageUsage",
            TelemetryInterface::BatteryStatus => "io.edgehog.devicemanager.BatteryStatus",
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
                })
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
