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
use astarte_device_sdk::AstarteAggregate;
use procfs::Current;

#[derive(Debug, AstarteAggregate)]
#[allow(non_snake_case)]
pub struct SystemStatus {
    pub availMemoryBytes: i64,
    pub bootId: String,
    pub taskCount: i32,
    pub uptimeMillis: i64,
}

/// get structured data for `io.edgehog.devicemanager.SystemStatus` interface
pub fn get_system_status() -> Result<SystemStatus, DeviceManagerError> {
    let meminfo = procfs::Meminfo::current()?;

    Ok(SystemStatus {
        availMemoryBytes: meminfo.mem_available.unwrap_or(0) as i64,
        bootId: procfs::sys::kernel::random::boot_id()?,
        taskCount: procfs::process::all_processes()?.count() as i32,
        uptimeMillis: procfs::Uptime::current()?.uptime_duration().as_millis() as i64,
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
        assert!(system_status.availMemoryBytes > 0);
        assert!(!system_status.bootId.is_empty());
        assert!(system_status.taskCount > 0);
        assert!(system_status.uptimeMillis > 0);
    }
}
