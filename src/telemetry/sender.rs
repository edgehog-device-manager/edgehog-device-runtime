// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Task to send the data.

use std::future::Future;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::telemetry::stats::storage_usage::StorageUsage;
use crate::telemetry::stats::system_status::SystemStatusTelemetry;
use crate::telemetry::stats::TelemetryInterface;
use crate::Client;

use super::stats::ContainerInterface;

pub(crate) trait TelemetryTask {
    fn send<C>(&mut self, client: &mut C) -> impl Future<Output = ()> + Send
    where
        C: Client + Send + Sync + 'static;
}

#[derive(Debug)]
pub struct Task<C> {
    client: C,
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

    pub(crate) fn spawn(
        client: C,
        cancel: CancellationToken,
        interface: TelemetryInterface,
        period: Duration,
        #[cfg(feature = "containers")] containers: &std::sync::Arc<
            tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>,
        >,
    ) where
        C: Client + Send + Sync + 'static,
    {
        tokio::spawn(Self::for_interface(
            client,
            cancel,
            interface,
            period,
            #[cfg(feature = "containers")]
            edgehog_containers::stats::StatsMonitor::new(std::sync::Arc::clone(containers)),
        ));
    }

    async fn for_interface(
        client: C,
        cancel: CancellationToken,
        interface: TelemetryInterface,
        period: Duration,
        #[cfg(feature = "containers")] containers: edgehog_containers::stats::StatsMonitor,
    ) where
        C: Client + Send + Sync + 'static,
    {
        let task = Task::new(client, interface, cancel, period);

        match interface {
            TelemetryInterface::SystemStatus => {
                let telemetry = SystemStatusTelemetry::default();

                task.run(telemetry).await;
            }
            TelemetryInterface::StorageUsage => {
                let telemetry = StorageUsage::default();

                task.run(telemetry).await;
            }
            TelemetryInterface::BatteryStatus => {
                cfg_if::cfg_if! {
                    if #[cfg(all(feature = "zbus", target_os = "linux"))] {
                        let telemetry = super::stats::battery_status::BatteryStatusTelemetry::default();

                        task.run(telemetry).await;
                    } else {
                        tracing::warn!("the battery status telemetry interface is not supported because the zbus feature is missing")
                    }
                }
            }
            TelemetryInterface::WiFiScanResults => {
                cfg_if::cfg_if! {
                    if #[cfg(feature = "wifiscanner")] {
                        let telemetry = super::stats::wifi_scan::WifiScan::default();

                        task.run(telemetry).await;
                    } else {
                        tracing::warn!("the wifi scan telemetry interface is not supported because the wifiscanner feature is missing")
                    }
                }
            }
            TelemetryInterface::ContainerBlkio => {
                task.container(
                    ContainerInterface::ContainerBlkio,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::ContainerCpu => {
                task.container(
                    ContainerInterface::ContainerCpu,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::ContainerMemory => {
                task.container(
                    ContainerInterface::ContainerMemory,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::ContainerMemoryStats => {
                task.container(
                    ContainerInterface::ContainerMemoryStats,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::ContainerNetworks => {
                task.container(
                    ContainerInterface::ContainerNetworks,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::ContainerProcesses => {
                task.container(
                    ContainerInterface::ContainerProcesses,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
            TelemetryInterface::VolumeUsage => {
                task.container(
                    ContainerInterface::VolumeUsage,
                    #[cfg(feature = "containers")]
                    containers,
                )
                .await
            }
        }
    }

    async fn container(
        self,
        interface: ContainerInterface,
        #[cfg(feature = "containers")] containers: edgehog_containers::stats::StatsMonitor,
    ) where
        C: Client + Send + Sync + 'static,
    {
        cfg_if::cfg_if! {
            if #[cfg(feature = "containers")] {
                let telemetry = super::stats::container::ContainerTelemetry::new(interface, containers);

                self.run(telemetry).await;
            } else {
                tracing::warn!("the {interface} telemetry interface is not supported because the container feature is not enabled")
            }
        }
    }

    pub async fn run<T>(mut self, mut telemetry: T)
    where
        C: Client + Send + Sync + 'static,
        T: TelemetryTask,
    {
        let mut interval = tokio::time::interval(self.period);

        while self
            .cancel
            .run_until_cancelled(interval.tick())
            .await
            .is_some()
        {
            info!(interface = %self.interface, "collecting telemetry",);

            telemetry.send(&mut self.client).await;
        }

        debug!(interface = %self.interface, "telemetry task cancelled");
    }
}
