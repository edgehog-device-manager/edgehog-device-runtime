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

use astarte_device_sdk::AstarteAggregate;
use std::collections::HashMap;
use sysinfo::Disks;
use tracing::{error, warn};

use crate::data::Publisher;

const INTERFACE: &str = "io.edgehog.devicemanager.StorageUsage";

#[derive(Debug, AstarteAggregate)]
#[astarte_aggregate(rename_all = "camelCase")]
pub struct DiskUsage {
    pub total_bytes: i64,
    pub free_bytes: i64,
}

#[derive(Debug)]
pub struct StorageUsage {
    disks: HashMap<String, DiskUsage>,
}

impl StorageUsage {
    /// Get structured data for `io.edgehog.devicemanager.StorageUsage` interface.
    ///
    /// The `/dev/` prefix is excluded from the device names since it is common for all devices.
    pub fn read() -> Self {
        let disks = Disks::new_with_refreshed_list();

        let disks = disks
            .list()
            .iter()
            .filter_map(|disk| {
                let name = disk.name().to_string_lossy();
                let name = name.strip_prefix("/dev/").unwrap_or(&name);

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
                    // Format to be send as aggregate object path
                    format!("/{name}"),
                    DiskUsage {
                        total_bytes,
                        free_bytes,
                    },
                ))
            })
            .collect();

        Self { disks }
    }

    /// Sends all the disks.
    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        for (path, v) in self.disks {
            if let Err(err) = client.send_object(INTERFACE, &path, v).await {
                error!(
                    "couldn't send {}: {}",
                    INTERFACE,
                    stable_eyre::Report::new(err)
                )
            }
        }
    }
}
