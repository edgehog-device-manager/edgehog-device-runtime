// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

use std::fs::Metadata;
use std::os::linux::fs::MetadataExt;

use tracing::error;

/// Returns the ID_FS_UUID_ENC udev device property or the `st_rdev` number
pub(crate) fn read_disk_uuid_prop(dev: &Metadata) -> String {
    let opt_device = udev::Device::from_devnum(udev::DeviceType::Block, dev.st_rdev())
        .inspect_err(|error| error!(%error, "couldn't get device from devnum"))
        .ok();

    let Some(device) = opt_device else {
        return format!("/{}", dev.st_rdev());
    };

    let Some(id) = device.property_value("ID_FS_UUID_ENC") else {
        error!("couldn't get udev UUID property");

        return format!("/{}", dev.st_rdev());
    };

    format!("/{}", id.display())
}
