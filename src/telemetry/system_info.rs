// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
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

use crate::data::{publish, Publisher};

use procfs::cmdline;
use std::env;
use tracing::{debug, error};

const INTERFACE: &str = "io.edgehog.devicemanager.SystemInfo";

#[derive(Debug, Default)]
pub struct SystemInfo {
    serial_number: Option<String>,
    part_number: Option<String>,
}

impl SystemInfo {
    pub fn read() -> Self {
        let mut serial_number: Option<String> = env::var("EDGEHOG_SYSTEM_SERIAL_NUMBER").ok();
        let mut part_number: Option<String> = env::var("EDGEHOG_SYSTEM_PART_NUMBER").ok();

        match cmdline() {
            Ok(cmdline) => {
                cmdline
                    .iter()
                    .filter_map(|line| line.split_once('='))
                    .for_each(|(k, v)| match k {
                        "edgehog_system_serial_number" => serial_number = Some(v.to_string()),
                        "edgehog_system_part_number" => part_number = Some(v.to_string()),
                        _ => {}
                    });
            }
            Err(err) => {
                error!(
                    "couldn't read the kernel cmd line: {}",
                    stable_eyre::Report::new(err)
                );
            }
        }

        Self {
            serial_number,
            part_number,
        }
    }

    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        if let Some(serial_number) = self.serial_number {
            publish(client, INTERFACE, "/serialNumber", serial_number).await;
        } else {
            debug!("missing serial number for {INTERFACE}");
        }

        if let Some(part_number) = self.part_number {
            publish(client, INTERFACE, "/partNumber", part_number).await;
        } else {
            debug!("missing part number for {INTERFACE}");
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteType;

    use crate::data::tests::MockPubSub;

    use super::*;

    #[test]
    fn get_system_info_test() {
        env::set_var("EDGEHOG_SYSTEM_SERIAL_NUMBER", "serial#");
        env::set_var("EDGEHOG_SYSTEM_PART_NUMBER", "part#");

        let sysinfo = SystemInfo::read();

        assert_eq!(sysinfo.serial_number.unwrap(), "serial#");
        assert_eq!(sysinfo.part_number.unwrap(), "part#");
    }

    #[tokio::test]
    async fn should_send_system_info() {
        let sysinfo = SystemInfo {
            serial_number: Some("serial".to_string()),
            part_number: Some("part".to_string()),
        };

        let mut client = MockPubSub::new();

        client
            .expect_send()
            .times(1)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.SystemInfo"
                    && path == "/serialNumber"
                    && *data == AstarteType::String("serial".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(1)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.SystemInfo"
                    && path == "/partNumber"
                    && *data == AstarteType::String("part".to_string())
            })
            .returning(|_, _, _| Ok(()));

        sysinfo.send(&client).await;
    }
}
