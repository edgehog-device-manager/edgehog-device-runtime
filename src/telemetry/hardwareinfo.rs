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
use astarte_sdk::types::AstarteType;
use std::collections::HashMap;

/// get structured data for `io.edgehog.devicemanager.HardwareInfo` interface
pub fn get_hardware_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    let uname = nix::sys::utsname::uname();
    ret.insert(
        "/cpu/architecture".to_owned(),
        uname.machine().to_string().into(),
    );

    let cpuinfo = procfs::CpuInfo::new()?;
    if let Some(f) = cpuinfo.fields.get("model") {
        ret.insert("/cpu/model".to_owned(), f.clone().into());
    }

    if let Some(f) = cpuinfo.fields.get("model name") {
        ret.insert("/cpu/modelName".to_owned(), f.clone().into());
    }

    if let Some(f) = cpuinfo.fields.get("vendor_id") {
        ret.insert("/cpu/vendor".to_owned(), f.clone().into());
    }

    let meminfo = procfs::Meminfo::new()?;
    ret.insert(
        "/mem/totalBytes".to_owned(),
        (meminfo.mem_total as i64).into(),
    );

    Ok(ret)
}
