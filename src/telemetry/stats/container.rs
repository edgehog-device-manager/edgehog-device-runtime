// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::fmt::Display;

use edgehog_containers::stats::StatsMonitor;

use crate::telemetry::sender::TelemetryTask;
use crate::Client;

#[derive(Debug)]
pub(crate) struct ContainerTelemetry {
    interface: ContainerInterface,
    stats: StatsMonitor,
}

impl ContainerTelemetry {
    pub(crate) fn new(interface: ContainerInterface, stats: StatsMonitor) -> Self {
        Self { interface, stats }
    }
}

impl TelemetryTask for ContainerTelemetry {
    #[allow(refining_impl_trait_internal)]
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send + Sync + 'static,
    {
        match self.interface {
            ContainerInterface::ContainerBlkio => {
                self.stats.blkio(client).await;
            }
            ContainerInterface::ContainerCpu => {
                self.stats.cpu(client).await;
            }
            ContainerInterface::ContainerMemory => {
                self.stats.memory(client).await;
            }
            ContainerInterface::ContainerMemoryStats => {
                self.stats.memory_stats(client).await;
            }
            ContainerInterface::ContainerNetworks => {
                self.stats.network(client).await;
            }
            ContainerInterface::ContainerProcesses => {
                self.stats.pids(client).await;
            }
            ContainerInterface::VolumeUsage => {
                self.stats.volumes(client).await;
            }
        }
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
