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

//! Container memory statistics

use std::collections::HashMap;

use astarte_device_sdk::IntoAstarteObject;

use super::Metric;

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerMemory {
    usage: i64,
    max_usage: i64,
    failcnt: i64,
    limit: i64,
}

impl Metric for ContainerMemory {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerMemory";
    const METRIC_NAME: &'static str = "container memory";
}

impl From<&bollard::models::ContainerMemoryStats> for ContainerMemory {
    fn from(value: &bollard::models::ContainerMemoryStats) -> Self {
        Self {
            usage: value
                .usage
                .unwrap_or_default()
                .try_into()
                .unwrap_or(i64::MAX),
            max_usage: value
                .max_usage
                .unwrap_or_default()
                .try_into()
                .unwrap_or(i64::MAX),
            failcnt: value
                .failcnt
                .unwrap_or_default()
                .try_into()
                .unwrap_or(i64::MAX),
            limit: value
                .limit
                .unwrap_or_default()
                .try_into()
                .unwrap_or(i64::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerMemoryStats {
    name: String,
    value: i64,
}

impl ContainerMemoryStats {
    pub(crate) fn from_stats(value: HashMap<String, u64>) -> Vec<Self> {
        value.into_iter().map(Self::from).collect()
    }
}

impl Metric for ContainerMemoryStats {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerMemoryStats";
    const METRIC_NAME: &'static str = "container memory statistics";
}

impl From<(String, u64)> for ContainerMemoryStats {
    fn from((name, value): (String, u64)) -> Self {
        Self {
            name,
            value: value.try_into().unwrap_or(i64::MAX),
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
    fn from_bollard_should_correctly_convert_stats() {
        let bollard_stats = bollard::models::ContainerMemoryStats {
            usage: Some(1024),
            max_usage: Some(2048),
            failcnt: Some(5),
            limit: Some(4096),
            ..Default::default()
        };

        let result = ContainerMemory::from(&bollard_stats);

        let expected = ContainerMemory {
            usage: 1024,
            max_usage: 2048,
            failcnt: 5,
            limit: 4096,
        };

        assert_eq!(result, expected);
    }

    #[test]
    fn from_bollard_should_handle_none_values() {
        let bollard_stats = bollard::models::ContainerMemoryStats {
            usage: None,
            max_usage: None,
            failcnt: None,
            limit: None,
            ..Default::default()
        };

        let result = ContainerMemory::from(&bollard_stats);

        let expected = ContainerMemory {
            usage: 0,
            max_usage: 0,
            failcnt: 0,
            limit: 0,
        };

        assert_eq!(result, expected);
    }

    #[test]
    fn from_bollard_should_saturate_at_i64_max() {
        let bollard_stats = bollard::models::ContainerMemoryStats {
            usage: Some(u64::MAX),
            max_usage: Some(u64::MAX),
            failcnt: Some(u64::MAX),
            limit: Some(u64::MAX),
            ..Default::default()
        };

        let result = ContainerMemory::from(&bollard_stats);

        let expected = ContainerMemory {
            usage: i64::MAX,
            max_usage: i64::MAX,
            failcnt: i64::MAX,
            limit: i64::MAX,
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
                predicate::eq("io.edgehog.devicemanager.apps.stats.ContainerMemory"),
                predicate::eq(format!("/{id}")),
                predicate::eq(AstarteObject::from_iter(
                    [
                        ("usage", AstarteData::LongInteger(1024)),
                        ("maxUsage", AstarteData::LongInteger(2048)),
                        ("failcnt", AstarteData::LongInteger(5)),
                        ("limit", AstarteData::LongInteger(4096)),
                    ]
                    .map(|(k, v)| (k.to_string(), v)),
                )),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        let stats = ContainerMemory {
            usage: 1024,
            max_usage: 2048,
            failcnt: 5,
            limit: 4096,
        };

        stats.send(&id, &mut client, &timestamp).await;
    }

    #[test]
    fn from_tuple_should_correctly_convert_stats() {
        let input = ("cache".to_string(), 1024);
        let result = ContainerMemoryStats::from(input);
        let expected = ContainerMemoryStats {
            name: "cache".to_string(),
            value: 1024,
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn from_stats_should_handle_multiple_entries() {
        let mut stats_map = HashMap::new();
        stats_map.insert("cache".to_string(), 1024);
        stats_map.insert("rss".to_string(), 2048);

        let mut result = ContainerMemoryStats::from_stats(stats_map);
        // Sort for predictable order in assertion
        result.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0],
            ContainerMemoryStats {
                name: "cache".to_string(),
                value: 1024
            }
        );
        assert_eq!(
            result[1],
            ContainerMemoryStats {
                name: "rss".to_string(),
                value: 2048
            }
        );
    }

    #[test]
    fn from_tuple_should_saturate_at_i64_max() {
        let input = ("total_inactive_file".to_string(), u64::MAX);
        let result = ContainerMemoryStats::from(input);
        let expected = ContainerMemoryStats {
            name: "total_inactive_file".to_string(),
            value: i64::MAX,
        };
        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn should_send_cgroup_v2_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        client
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.stats.ContainerMemoryStats"),
                predicate::eq(format!("/{id}")),
                predicate::eq(AstarteObject::from_iter(
                    [
                        ("name", AstarteData::String("rss".to_string())),
                        ("value", AstarteData::LongInteger(4096)),
                    ]
                    .map(|(k, v)| (k.to_string(), v)),
                )),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        let stats = ContainerMemoryStats {
            name: "rss".to_string(),
            value: 4096,
        };

        stats.send(&id, &mut client, &timestamp).await;
    }
}
