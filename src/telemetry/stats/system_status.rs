// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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

use std::collections::HashSet;
use std::time::Duration;

use astarte_device_sdk::IntoAstarteObject;
use astarte_device_sdk::chrono::Utc;
use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, RefreshKind, System};
use tracing::{debug, error};

use crate::Client;
use crate::data::send_object_with_timestamp;
use crate::telemetry::sender::TelemetryTask;

pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.SystemStatus";

#[derive(Debug, Clone, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub struct SystemStatus {
    pub avail_memory_bytes: i64,
    pub boot_id: String,
    pub task_count: i32,
    pub uptime_millis: i64,
}

impl SystemStatus {
    /// Get structured data for `io.edgehog.devicemanager.SystemStatus` interface
    ///
    /// The fields that errors or have an invalid value (too big to be sent to Astarte), will be
    /// set to 0 or empty as a null/default value.
    pub fn read() -> Option<Self> {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::nothing().with_ram())
                .with_processes(ProcessRefreshKind::nothing().with_tasks()),
        );

        let mem_avail = system.available_memory();
        let avail_memory_bytes = i64::try_from(mem_avail)
            .inspect_err(|_| error!(mem_avail, "value to big to send as i64"))
            .unwrap_or_default();

        cfg_if::cfg_if! {
            if #[cfg(any(target_os = "linux", target_os = "android"))] {
                let boot_id = procfs::sys::kernel::random::boot_id().unwrap_or_else(|err| {
                    error!(error = %err, "couldn't get the boot_id");

                    String::new()
                });
            } else {
                let boot_id = String::new();
            }
        };

        let task_count = system
            .processes()
            .values()
            .map(|procs| {
                procs.tasks().map(HashSet::len).unwrap_or_else(|| {
                    debug!("missing tasks for process");

                    // Count at least the process
                    1
                })
            })
            .map(|procs| {
                i32::try_from(procs).unwrap_or_else(|_| {
                    error!("task_count to big to send as i32: {procs}");

                    0
                })
            })
            .sum();

        let uptime = System::uptime();
        let uptime_millis =
            i64::try_from(Duration::from_secs(uptime).as_millis()).unwrap_or_else(|_| {
                error!(uptime_secs = uptime, "to big to send as millis i64");

                0
            });

        Some(SystemStatus {
            avail_memory_bytes,
            boot_id,
            task_count,
            uptime_millis,
        })
    }
}

#[derive(Debug, Default)]
pub(crate) struct SystemStatusTelemetry {}

impl TelemetryTask for SystemStatusTelemetry {
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send + Sync + 'static,
    {
        let Some(status) = SystemStatus::read() else {
            return;
        };

        send_object_with_timestamp(client, INTERFACE, "/systemStatus", status, Utc::now()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_system_status_test() {
        let system_status = SystemStatus::read().unwrap();

        #[cfg(any(target_os = "linux", target_os = "android"))]
        assert!(!system_status.boot_id.is_empty());

        assert!(system_status.avail_memory_bytes > 0);
        assert!(system_status.task_count > 0);
        assert!(system_status.uptime_millis > 0);
    }
}
