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
use std::collections::HashMap;
use sysinfo::{DiskExt, System, SystemExt};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    pub total_bytes: i64,
    pub free_bytes: i64,
}

/// get structured data for `io.edgehog.devicemanager.StorageUsage` interface
/// /dev/ is excluded from the device names since it is common for all devices
pub fn get_storage_usage() -> Result<HashMap<String, DiskUsage>, DeviceManagerError> {
    let mut ret: HashMap<String, DiskUsage> = HashMap::new();
    let mut sys = System::new_all();
    sys.refresh_disks();

    for disk in sys.disks() {
        let disk_name = disk.name().to_str().unwrap().replace("/dev/", "");
        ret.insert(
            disk_name.to_owned() as String,
            DiskUsage {
                total_bytes: disk.total_space() as i64,
                free_bytes: disk.available_space() as i64,
            },
        );
    }
    Ok(ret)
}
