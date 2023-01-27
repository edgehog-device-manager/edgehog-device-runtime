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
use astarte_device_sdk::types::AstarteType;
use procfs::cmdline;
use std::collections::HashMap;
use std::env;

pub fn get_system_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let cmdline_params = cmdline().unwrap();
    let mut seral_number: Option<String> = env::var("EDGEHOG_SYSTEM_SERIAL_NUMBER").ok();
    let mut part_number: Option<String> = env::var("EDGEHOG_SYSTEM_PART_NUMBER").ok();
    for param in cmdline_params.iter() {
        if seral_number.is_none() && param.starts_with("edgehog_system_serial_number") {
            let first_half = format!("{}=", "edgehog_system_serial_number");
            seral_number = Some(param.replace(first_half.as_str(), ""));
        }
        if part_number.is_none() && param.starts_with("edgehog_system_part_number") {
            let first_half = format!("{}=", "edgehog_system_part_number");
            part_number = Some(param.replace(first_half.as_str(), ""));
        }
    }
    let mut ret: HashMap<String, AstarteType> = HashMap::new();
    if let Some(f) = seral_number {
        ret.insert("/serialNumber".to_owned(), f.into());
    }
    if let Some(f) = part_number {
        ret.insert("/partNumber".to_owned(), f.into());
    }
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use crate::telemetry::system_info::get_system_info;
    use astarte_device_sdk::types::AstarteType;
    use std::env;

    #[test]
    fn get_system_info_test() {
        env::set_var("EDGEHOG_SYSTEM_SERIAL_NUMBER", "serial#");
        env::set_var("EDGEHOG_SYSTEM_PART_NUMBER", "part#");

        let sysinfo = get_system_info();
        assert!(sysinfo.is_ok());
        let sysinfo_map = sysinfo.unwrap();
        assert_eq!(
            sysinfo_map.get("/serialNumber").unwrap().to_owned(),
            AstarteType::String("serial#".to_string())
        );
        assert_eq!(
            sysinfo_map.get("/partNumber").unwrap().to_owned(),
            AstarteType::String("part#".to_string())
        );
    }
}
