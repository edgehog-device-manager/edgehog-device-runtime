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

use std::{collections::HashMap, fmt::Display};

use astarte_device_sdk::types::AstarteType;
use log::warn;

use crate::error::DeviceManagerError;

#[derive(Debug)]
enum TechnologyType {
    Ethernet,
    Cellular,
    WiFi,
}

impl Display for TechnologyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TechnologyType::Ethernet => write!(f, "Ethernet"),
            TechnologyType::Cellular => write!(f, "Cellular"),
            TechnologyType::WiFi => write!(f, "WiFi"),
        }
    }
}

#[derive(Debug)]
struct NetworkInterfaceProperties {
    interface: String,
    mac_address: String,
    technology_type: TechnologyType,
}

fn get_supported_network_interfaces() -> Result<Vec<NetworkInterfaceProperties>, DeviceManagerError>
{
    const ARPHRD_ETHER: &str = "1";
    const ARPHRD_PPP: &str = "512";

    let mut results = Vec::new();

    let mut enumerator = udev::Enumerator::new()?;

    enumerator.match_subsystem("net")?;

    for device in enumerator.scan_devices()? {
        if device.property_value("ID_BUS").is_none() {
            // This is a virtual device
            continue;
        }

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
    let supported_networks_interfaces = get_supported_network_interfaces().unwrap_or_else(|err| {
        warn!("{err}");
        Default::default()
    });

    Ok(network_interface_to_astarte(supported_networks_interfaces))
}

fn network_interface_to_astarte(
    eth_wifi: Vec<NetworkInterfaceProperties>,
) -> HashMap<String, AstarteType> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    for iff in eth_wifi {
        ret.insert(
            format!("/{}/macAddress", iff.interface),
            AstarteType::String(iff.mac_address.to_ascii_lowercase()),
        );
        ret.insert(
            format!("/{}/technologyType", iff.interface),
            AstarteType::String(iff.technology_type.to_string()),
        );
    }

    ret
}

#[cfg(test)]
mod tests {
    use crate::telemetry::net_if_properties::{
        get_supported_network_interfaces, network_interface_to_astarte, NetworkInterfaceProperties,
        TechnologyType,
    };
    use astarte_device_sdk::types::AstarteType;

    #[test]
    fn technology_type_to_string_test() {
        assert_eq!(TechnologyType::Ethernet.to_string(), "Ethernet".to_string());
        assert_eq!(TechnologyType::Cellular.to_string(), "Cellular".to_string());
        assert_eq!(TechnologyType::WiFi.to_string(), "WiFi".to_string());
    }

    #[test]
    fn network_interface_to_astarte_test() {
        let eth_wifi = vec![
            NetworkInterfaceProperties {
                interface: "wifi_test".to_string(),
                mac_address: "00:11:22:33:44:55".to_string(),
                technology_type: TechnologyType::WiFi,
            },
            NetworkInterfaceProperties {
                interface: "eth_test".to_string(),
                mac_address: "11:22:33:44:55:66".to_string(),
                technology_type: TechnologyType::Ethernet,
            },
            NetworkInterfaceProperties {
                interface: "cellular_test".to_string(),
                mac_address: "22:33:44:55:66:77".to_string(),
                technology_type: TechnologyType::Cellular,
            },
        ];

        let astarte_payload = network_interface_to_astarte(eth_wifi);

        assert_eq!(
            astarte_payload.get("/wifi_test/macAddress").unwrap(),
            &AstarteType::String("00:11:22:33:44:55".to_string())
        );
        assert_eq!(
            astarte_payload.get("/wifi_test/technologyType").unwrap(),
            &AstarteType::String("WiFi".to_string())
        );
        assert_eq!(
            astarte_payload.get("/eth_test/macAddress").unwrap(),
            &AstarteType::String("11:22:33:44:55:66".to_string())
        );
        assert_eq!(
            astarte_payload.get("/eth_test/technologyType").unwrap(),
            &AstarteType::String("Ethernet".to_string())
        );
        assert_eq!(
            astarte_payload.get("/cellular_test/macAddress").unwrap(),
            &AstarteType::String("22:33:44:55:66:77".to_string())
        );
        assert_eq!(
            astarte_payload
                .get("/cellular_test/technologyType")
                .unwrap(),
            &AstarteType::String("Cellular".to_string())
        );
    }

    #[test]
    fn get_supported_network_interfaces_run_test() {
        assert!(get_supported_network_interfaces().is_ok());
    }
}
