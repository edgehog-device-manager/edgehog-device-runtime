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

use astarte_device_sdk::{astarte_aggregate, AstarteAggregate};
use log::{error, warn};
use std::collections::HashMap;
use sysinfo::{DiskExt, System, SystemExt};

#[derive(Debug, AstarteAggregate)]
#[astarte_aggregate(rename_all = "camelCase")]
pub struct DiskUsage {
    pub total_bytes: i64,
    pub free_bytes: i64,
}

/// get structured data for `io.edgehog.devicemanager.StorageUsage` interface
/// /dev/ is excluded from the device names since it is common for all devices
pub fn get_storage_usage() -> HashMap<String, DiskUsage> {
    let mut sys = System::new_all();
    sys.refresh_disks();

    sys.disks()
        .iter()
        .filter_map(|disk| {
            let Some(name) = disk.name().to_str() else {
                warn!("non-utf8 path {}, ignoring", disk.name().to_string_lossy());
                return None;
            };
            let name = name.strip_prefix("/dev/").unwrap_or(name);
            // remove disks with a higher depth
            if name.contains('/') {
                warn!("not simple disks device, ignoring");
                return None;
            }
            let Ok(total_bytes) = disk.total_space().try_into() else {
                error!("disk size too big, ignoring");
                return None;
            };
            let Ok(free_bytes) = disk.available_space().try_into() else {
                error!("available space too big, ignoring");
                return None;
            };
            Some((
                name.to_string(),
                DiskUsage {
                    total_bytes,
                    free_bytes,
                },
            ))
        })
        .collect()
}
