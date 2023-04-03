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

use crate::DeviceManagerError;
use astarte_device_sdk::AstarteAggregate;
use wifiscanner::Wifi;

#[derive(Debug, AstarteAggregate, PartialEq)]
#[allow(non_snake_case)]
pub struct WifiScanResult {
    channel: i32,
    connected: bool,
    essid: String,
    macAddress: String,
    rssi: i32,
}

pub fn get_wifi_scan_results() -> Result<Vec<WifiScanResult>, DeviceManagerError> {
    let mut ret = Vec::new();
    if let Ok(wifi_array) = wifiscanner::scan() {
        for wifi in wifi_array {
            let w = wifi.try_into()?;
            ret.push(w);
        }
    }

    Ok(ret)
}

impl TryFrom<Wifi> for WifiScanResult {
    type Error = DeviceManagerError;
    fn try_from(wifi: Wifi) -> Result<Self, Self::Error> {
        Ok(WifiScanResult {
            channel: wifi.channel.parse::<i32>()?,
            connected: false,
            essid: wifi.ssid,
            macAddress: wifi.mac,
            rssi: wifi.signal_level.parse::<i32>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::telemetry::wifi_scan::{get_wifi_scan_results, WifiScanResult};
    use wifiscanner::Wifi;

    #[test]
    fn wifi_scan_test() {
        assert!(get_wifi_scan_results().is_ok());
    }

    #[test]
    fn get_hashmap_from_wifi_test() {
        let wifi = Wifi {
            mac: "ab:cd:ef:01:23:45".to_string(),
            ssid: "Vodafone Hotspot".to_string(),
            channel: "6".to_string(),
            signal_level: "-92".to_string(),
            security: "Open".to_string(),
        };
        let inter: Result<WifiScanResult, _> = wifi.try_into();
        assert!(inter.is_ok());
        assert_eq!(
            inter.unwrap(),
            WifiScanResult {
                channel: 6,
                connected: false,
                essid: "Vodafone Hotspot".to_string(),
                macAddress: "ab:cd:ef:01:23:45".to_string(),
                rssi: -92
            }
        );
    }
}
