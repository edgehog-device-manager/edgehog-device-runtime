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

use crate::data::Publisher;
use astarte_device_sdk::AstarteAggregate;
use stable_eyre::eyre::Context;
use tracing::error;
use wifiscanner::Wifi;

const INTERFACE: &str = "io.edgehog.devicemanager.WiFiScanResults";

#[derive(Debug, AstarteAggregate, PartialEq)]
#[astarte_aggregate(rename_all = "camelCase")]
pub struct WifiScanResult {
    channel: i32,
    connected: bool,
    essid: String,
    mac_address: String,
    rssi: i32,
}

pub async fn send_wifi_scan<T>(client: &T)
where
    T: Publisher,
{
    let res = tokio::task::spawn_blocking(|| {
        wifiscanner::scan().unwrap_or_else(|err| {
            // wifiscanner::Error doesn't impl Display
            error!("couldn't get wifi networks: {err:?}",);

            Vec::new()
        })
    })
    .await;

    let networks = match res {
        Ok(networks) => networks,
        Err(err) => {
            error!(
                "couldn't get wifi networks: {}",
                stable_eyre::Report::new(err)
            );

            return;
        }
    };

    let iter = networks
        .into_iter()
        .filter_map(|wifi| match WifiScanResult::try_from(wifi) {
            Ok(value) => Some(value),
            Err(err) => {
                error!("couldn't get wifi scan information: {err}");

                None
            }
        });

    for scan in iter {
        if let Err(err) = client.send_object(INTERFACE, "/ap", scan).await {
            error!(
                "couldn't send {}: {}",
                INTERFACE,
                stable_eyre::Report::new(err)
            );
        }
    }
}

impl TryFrom<Wifi> for WifiScanResult {
    type Error = stable_eyre::Report;

    fn try_from(wifi: Wifi) -> Result<Self, Self::Error> {
        let channel = wifi
            .channel
            .parse()
            .wrap_err_with(|| format!("channel value is not a valid i32: {}", wifi.channel))?;

        let rssi = wifi
            .signal_level
            .parse()
            .wrap_err_with(|| format!("rssi value is not a valid i32: {}", wifi.channel))?;

        Ok(WifiScanResult {
            channel,
            connected: false,
            essid: wifi.ssid,
            mac_address: wifi.mac,
            rssi,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::data::tests::MockPubSub;

    use super::*;

    #[tokio::test]
    async fn wifi_scan_test() {
        let mut client = MockPubSub::new();

        client
            .expect_send_object::<WifiScanResult>()
            .times(..)
            .withf(|interface, path, _| {
                interface == "io.edgehog.devicemanager.WiFiScanResults" && path == "/ap"
            })
            .returning(|_, _, _| Ok(()));

        send_wifi_scan(&client).await;
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
        let inter = WifiScanResult::try_from(wifi).unwrap();
        assert_eq!(
            inter,
            WifiScanResult {
                channel: 6,
                connected: false,
                essid: "Vodafone Hotspot".to_string(),
                mac_address: "ab:cd:ef:01:23:45".to_string(),
                rssi: -92
            }
        );
    }
}
