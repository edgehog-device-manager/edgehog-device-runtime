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
use log::warn;
use std::collections::HashMap;

#[derive(Debug)]
enum TechnologyType {
    Ethernet,
    Cellular,
    WiFi,
}

impl ToString for TechnologyType {
    fn to_string(&self) -> String {
        match self {
            TechnologyType::Ethernet => "Ethernet",
            TechnologyType::Cellular => "Cellular",
            TechnologyType::WiFi => "WiFi",
        }
        .to_owned()
    }
}

#[derive(Debug)]
struct NetworkInterfaceProperties {
    interface: String,
    mac_address: String,
    technology_type: TechnologyType,
}

fn get_ethernet_wifi() -> Result<Vec<NetworkInterfaceProperties>, DeviceManagerError> {
    const ARPHRD_ETHER: &str = "1";
    const ARPHRD_PPP: &str = "512";

    let mut results = Vec::new();

    let mut enumerator = udev::Enumerator::new()?;

    enumerator.match_subsystem("net")?;

    for device in enumerator.scan_devices()? {
        let (address, type_) = match (
            device.attribute_value("address"),
            device.attribute_value("type"),
        ) {
            (Some(addr), Some(type_)) => (addr, type_),
            _ => continue,
        };

        let technology_type = match type_.to_string_lossy().trim() {
            ARPHRD_ETHER => {
                let uevent = device
                    .attribute_value("uevent")
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if uevent.contains("DEVTYPE=wlan") {
                    TechnologyType::WiFi
                } else if uevent.contains("DEVTYPE=bridge") {
                    continue;
                } else {
                    TechnologyType::Ethernet
                }
            }

            ARPHRD_PPP => TechnologyType::Cellular,

            _ => {
                continue;
            }
        };

        results.push(NetworkInterfaceProperties {
            interface: device.sysname().to_string_lossy().into_owned(),
            mac_address: address.to_string_lossy().into_owned(),
            technology_type,
        });
    }

    Ok(results)
}

/// get structured data for `io.edgehog.devicemanager.NetworkInterfaceProperties` interface
pub async fn get_network_interface_properties(
) -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    let eth_wifi = get_ethernet_wifi().map_or_else(
        |err| {
            warn!("{err}");
            Default::default()
        },
        |d| d,
    );

    for iff in eth_wifi {
        ret.insert(
            format!("/{}/macAddress", iff.interface),
            astarte_sdk::types::AstarteType::String(iff.mac_address.to_ascii_lowercase()),
        );
        ret.insert(
            format!("/{}/technologyType", iff.interface),
            astarte_sdk::types::AstarteType::String(iff.technology_type.to_string()),
        );
    }

    Ok(ret)
}
