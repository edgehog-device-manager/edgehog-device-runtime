// This file is part of Edgehog.
//
// Copyright 2022 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use astarte_device_sdk::chrono::Utc;
use astarte_device_sdk::{Client, IntoAstarteObject};
use stable_eyre::eyre::Context;
use tracing::error;
use wifiscanner::Wifi;

use crate::data::send_object_with_timestamp;
use crate::telemetry::sender::TelemetryTask;

pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.WiFiScanResults";

#[derive(Debug, IntoAstarteObject, PartialEq)]
#[astarte_object(rename_all = "camelCase")]
pub struct WifiScanResult {
    channel: i32,
    connected: bool,
    essid: String,
    mac_address: String,
    rssi: i32,
}

impl WifiScanResult {
    async fn read() -> impl Iterator<Item = WifiScanResult> {
        tokio::task::spawn_blocking(|| {
            wifiscanner::scan().unwrap_or_else(|err| {
                // wifiscanner::Error doesn't impl Display
                error!(error = ?err, "couldn't get wifi networks");

                Vec::new()
            })
        })
        .await
        .unwrap_or_else(|err| {
            error!(
                "couldn't get wifi networks: {}",
                stable_eyre::Report::new(err)
            );

            Vec::new()
        })
        .into_iter()
        .filter_map(|wifi| match WifiScanResult::try_from(wifi) {
            Ok(value) => Some(value),
            Err(err) => {
                error!("couldn't get wifi scan information: {err}");

                None
            }
        })
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

#[derive(Debug, Default)]
pub(crate) struct WifiScan {}

impl TelemetryTask for WifiScan {
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send,
    {
        for scan in WifiScanResult::read().await {
            send_object_with_timestamp(client, INTERFACE, "/ap", scan, Utc::now()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;

    use super::*;

    #[tokio::test]
    async fn wifi_scan_test() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_send_object_with_timestamp()
            .times(..)
            .with(
                predicate::eq("io.edgehog.devicemanager.WiFiScanResults"),
                predicate::eq("/ap"),
                predicate::always(),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        WifiScan::default().send(&mut client).await;
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
