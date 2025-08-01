// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::collections::HashMap;

use astarte_device_sdk::IntoAstarteObject;

use super::Metric;

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerNetworkStats {
    interface: String,
    rx_bytes: i64,
    rx_dropped: i64,
    rx_errors: i64,
    rx_packets: i64,
    tx_bytes: i64,
    tx_dropped: i64,
    tx_errors: i64,
    tx_packets: i64,
}

impl ContainerNetworkStats {
    pub(crate) fn from_stats(
        value: HashMap<String, bollard::models::ContainerNetworkStats>,
    ) -> Vec<Self> {
        value.into_iter().map(Self::from).collect()
    }
}

impl Metric for ContainerNetworkStats {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerNetworks";
    const METRIC_NAME: &'static str = "container network";
}

impl From<(String, bollard::models::ContainerNetworkStats)> for ContainerNetworkStats {
    fn from((interface, stats): (String, bollard::models::ContainerNetworkStats)) -> Self {
        Self {
            interface,
            rx_bytes: stats
                .rx_bytes
                .map(|rx_bytes| rx_bytes.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            rx_dropped: stats
                .rx_dropped
                .map(|rx_dropped| rx_dropped.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            rx_errors: stats
                .rx_errors
                .map(|rx_dropped| rx_dropped.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            rx_packets: stats
                .rx_packets
                .map(|rx_packets| rx_packets.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            tx_bytes: stats
                .tx_bytes
                .map(|tx_bytes| tx_bytes.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            tx_dropped: stats
                .tx_dropped
                .map(|tx_dropped| tx_dropped.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            tx_errors: stats
                .tx_errors
                .map(|tx_errors| tx_errors.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
            tx_packets: stats
                .tx_packets
                .map(|tx_packets| tx_packets.try_into().unwrap_or(i64::MAX))
                .unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{predicate, Sequence};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn from_tuple_should_correctly_convert_stats() {
        let interface_name = "eth0".to_string();
        let bollard_stats = bollard::models::ContainerNetworkStats {
            rx_dropped: Some(1),
            rx_bytes: Some(1024),
            rx_errors: Some(2),
            tx_packets: Some(20),
            tx_dropped: Some(3),
            rx_packets: Some(10),
            tx_errors: Some(4),
            tx_bytes: Some(2048),
            endpoint_id: None,
            instance_id: None,
        };
        let input = (interface_name.clone(), bollard_stats);

        let result = ContainerNetworkStats::from(input);

        let expected = ContainerNetworkStats {
            interface: interface_name,
            rx_bytes: 1024,
            rx_dropped: 1,
            rx_errors: 2,
            rx_packets: 10,
            tx_bytes: 2048,
            tx_dropped: 3,
            tx_errors: 4,
            tx_packets: 20,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn from_stats_should_handle_multiple_interfaces() {
        let mut stats_map = HashMap::new();
        stats_map.insert(
            "eth0".to_string(),
            bollard::models::ContainerNetworkStats {
                rx_dropped: Some(2),
                rx_bytes: Some(100),
                rx_errors: Some(1),
                tx_packets: Some(20),
                tx_dropped: Some(4),
                rx_packets: Some(10),
                tx_errors: Some(3),
                tx_bytes: Some(200),
                endpoint_id: None,
                instance_id: None,
            },
        );
        stats_map.insert(
            "lo".to_string(),
            bollard::models::ContainerNetworkStats {
                rx_dropped: Some(6),
                rx_bytes: Some(300),
                rx_errors: Some(5),
                tx_packets: Some(50),
                tx_dropped: Some(8),
                rx_packets: Some(30),
                tx_errors: Some(7),
                tx_bytes: Some(500),
                endpoint_id: None,
                instance_id: None,
            },
        );

        let mut result = ContainerNetworkStats::from_stats(stats_map);
        result.sort_by(|a, b| a.interface.cmp(&b.interface));

        assert_eq!(result.len(), 2);

        let expected_eth0 = ContainerNetworkStats {
            interface: "eth0".to_string(),
            rx_bytes: 100,
            rx_dropped: 2,
            rx_errors: 1,
            rx_packets: 10,
            tx_bytes: 200,
            tx_dropped: 4,
            tx_errors: 3,
            tx_packets: 20,
        };
        assert_eq!(result[0], expected_eth0);

        let expected_lo = ContainerNetworkStats {
            interface: "lo".to_string(),
            rx_bytes: 300,
            rx_dropped: 6,
            rx_errors: 5,
            rx_packets: 30,
            tx_bytes: 500,
            tx_dropped: 8,
            tx_errors: 7,
            tx_packets: 50,
        };
        assert_eq!(result[1], expected_lo);
    }

    #[test]
    fn from_tuple_should_saturate_at_i64_max() {
        let interface_name = "eth0".to_string();
        let bollard_stats = bollard::models::ContainerNetworkStats {
            rx_dropped: Some(u64::MAX),
            rx_bytes: Some(u64::MAX),
            rx_errors: Some(u64::MAX),
            tx_packets: Some(u64::MAX),
            tx_dropped: Some(u64::MAX),
            rx_packets: Some(u64::MAX),
            tx_errors: Some(u64::MAX),
            tx_bytes: Some(u64::MAX),
            ..Default::default()
        };
        let input = (interface_name.clone(), bollard_stats);

        let result = ContainerNetworkStats::from(input);

        let expected = ContainerNetworkStats {
            interface: interface_name,
            rx_bytes: i64::MAX,
            rx_dropped: i64::MAX,
            rx_errors: i64::MAX,
            rx_packets: i64::MAX,
            tx_bytes: i64::MAX,
            tx_dropped: i64::MAX,
            tx_errors: i64::MAX,
            tx_packets: i64::MAX,
        };
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn should_send_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        client
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.stats.ContainerNetworks"),
                predicate::eq(format!("/{id}")),
                predicate::eq(AstarteObject::from_iter(
                    [
                        ("interface", AstarteData::from("lo")),
                        ("rxBytes", AstarteData::LongInteger(300)),
                        ("rxDropped", AstarteData::LongInteger(6)),
                        ("rxErrors", AstarteData::LongInteger(5)),
                        ("rxPackets", AstarteData::LongInteger(30)),
                        ("txBytes", AstarteData::LongInteger(500)),
                        ("txDropped", AstarteData::LongInteger(8)),
                        ("txErrors", AstarteData::LongInteger(7)),
                        ("txPackets", AstarteData::LongInteger(50)),
                    ]
                    .map(|(k, v)| (k.to_string(), v)),
                )),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        let stats = ContainerNetworkStats {
            interface: "lo".to_string(),
            rx_bytes: 300,
            rx_dropped: 6,
            rx_errors: 5,
            rx_packets: 30,
            tx_bytes: 500,
            tx_dropped: 8,
            tx_errors: 7,
            tx_packets: 50,
        };

        stats.send(&id, &mut client, &timestamp).await;
    }
}
