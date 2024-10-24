// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Create image request

use astarte_device_sdk::FromEvent;

use crate::volume::Volume;

use super::{parse_kv_map, ReqError};

/// Request to pull a Docker Volume.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateVolumeRequest",
    path = "/volume",
    rename_all = "camelCase"
)]
pub(crate) struct CreateVolume {
    pub(crate) id: String,
    pub(crate) driver: String,
    pub(crate) options: Vec<String>,
}

impl TryFrom<CreateVolume> for Volume<String> {
    type Error = ReqError;

    fn try_from(value: CreateVolume) -> Result<Self, Self::Error> {
        let driver = if value.driver.is_empty() {
            String::from("local")
        } else {
            value.driver
        };

        let driver_opts = parse_kv_map(&value.options)?;

        Ok(Volume {
            name: value.id,
            driver,
            driver_opts,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashMap;

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};
    use itertools::Itertools;

    use super::*;

    pub fn create_volume_request_event(id: &str, driver: &str, options: &[&str]) -> DeviceEvent {
        let options = options.iter().map(|s| s.to_string()).collect_vec();

        let fields = [
            ("id".to_string(), AstarteType::String(id.to_string())),
            (
                "driver".to_string(),
                AstarteType::String(driver.to_string()),
            ),
            ("options".to_string(), AstarteType::StringArray(options)),
        ]
        .into_iter()
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateVolumeRequest".to_string(),
            path: "/volume".to_string(),
            data: Value::Object(fields),
        }
    }

    #[test]
    fn create_volume_request() {
        let event = create_volume_request_event("id", "driver", &["foo=bar", "some="]);

        let request = CreateVolume::from_event(event).unwrap();

        let expect = CreateVolume {
            id: "id".to_string(),
            driver: "driver".to_string(),
            options: ["foo=bar", "some="].map(str::to_string).to_vec(),
        };

        assert_eq!(request, expect);
    }

    #[test]
    fn volume_default_driver() {
        let event = create_volume_request_event("id", "", &["foo=bar", "some="]);

        let request = CreateVolume::from_event(event).unwrap();

        let expect = Volume {
            name: "id",
            driver: "local",
            driver_opts: HashMap::from([("foo".to_string(), "bar"), ("some".to_string(), "")]),
        };

        let vol = Volume::try_from(request).unwrap();

        assert_eq!(vol, expect);
    }
}
