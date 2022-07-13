/*
 * This file is part of Edgehog.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::error::DeviceManagerError;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub avail_memory_bytes: i64,
    pub boot_id: String,
    pub task_count: i32,
    pub uptime_millis: i64,
}

/// get structured data for `io.edgehog.devicemanager.SystemStatus` interface
pub fn get_system_status() -> Result<SystemStatus, DeviceManagerError> {
    let meminfo = procfs::Meminfo::new()?;

    Ok(SystemStatus {
        avail_memory_bytes: meminfo.mem_available.unwrap_or(0) as i64,
        boot_id: procfs::sys::kernel::random::boot_id()?,
        task_count: procfs::process::all_processes()?.count() as i32,
        uptime_millis: procfs::Uptime::new()?.uptime_duration().as_millis() as i64,
    })
}

#[cfg(test)]
mod tests {
    use crate::telemetry::system_status::get_system_status;

    #[test]
    fn get_system_status_test() {
        let system_status_result = get_system_status();
        assert!(system_status_result.is_ok());

        let system_status = system_status_result.unwrap();
        assert!(system_status.avail_memory_bytes > 0);
        assert!(!system_status.boot_id.is_empty());
        assert!(system_status.task_count > 0);
        assert!(system_status.uptime_millis > 0);
    }
}
