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

//! Container and volume storage stats.

use astarte_device_sdk::IntoAstarteObject;
use bollard::secret::{ContainerBlkioStatEntry, ContainerBlkioStats};

use super::{IntoAstarteExt, Metric};

macro_rules! push_stats {
    ($vec:ident, $stat:ident) => {
        ContainerBlkio::push_stats(&mut $vec, stringify!($stat), $stat);
    };
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerBlkio {
    name: &'static str,
    major: i64,
    minor: i64,
    op: String,
    value: i64,
}

impl ContainerBlkio {
    pub(crate) fn from_stats(
        ContainerBlkioStats {
            io_service_bytes_recursive,
            io_serviced_recursive,
            io_queue_recursive,
            io_service_time_recursive,
            io_wait_time_recursive,
            io_merged_recursive,
            io_time_recursive,
            sectors_recursive,
        }: ContainerBlkioStats,
    ) -> Vec<Self> {
        let mut stats = Vec::new();

        push_stats!(stats, io_service_bytes_recursive);
        push_stats!(stats, io_serviced_recursive);
        push_stats!(stats, io_queue_recursive);
        push_stats!(stats, io_service_time_recursive);
        push_stats!(stats, io_wait_time_recursive);
        push_stats!(stats, io_merged_recursive);
        push_stats!(stats, io_time_recursive);
        push_stats!(stats, sectors_recursive);

        stats
    }

    fn push_stats(
        stats: &mut Vec<Self>,
        name: &'static str,
        values: Option<Vec<ContainerBlkioStatEntry>>,
    ) {
        let Some(values) = values else { return };

        stats.extend(values.into_iter().map(|entry| Self {
            name,
            major: entry.major.into_astarte(),
            minor: entry.minor.into_astarte(),
            op: entry.op.unwrap_or_default(),
            value: entry.value.into_astarte(),
        }))
    }
}

impl Metric for ContainerBlkio {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerBlkio";
    const METRIC_NAME: &'static str = "container blkio";
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk_mock::MockDeviceClient;
    use bollard::secret::{ContainerBlkioStatEntry, ContainerBlkioStats};
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn should_convert_from_bollard() {
        let empty_bollard_stats = ContainerBlkioStats {
            io_service_bytes_recursive: None,
            io_serviced_recursive: None,
            io_queue_recursive: None,
            io_service_time_recursive: None,
            io_wait_time_recursive: None,
            io_merged_recursive: None,
            io_time_recursive: None,
            sectors_recursive: None,
        };
        assert!(ContainerBlkio::from_stats(empty_bollard_stats).is_empty());

        let bollard_stats = ContainerBlkioStats {
            io_service_bytes_recursive: Some(vec![
                ContainerBlkioStatEntry {
                    major: Some(8),
                    minor: Some(0),
                    op: Some("read".to_string()),
                    value: Some(1024),
                },
                ContainerBlkioStatEntry {
                    major: Some(8),
                    minor: Some(0),
                    op: Some("write".to_string()),
                    value: Some(2048),
                },
            ]),
            io_serviced_recursive: Some(vec![ContainerBlkioStatEntry {
                major: Some(252),
                minor: Some(1),
                op: Some("total".to_string()),
                value: Some(5),
            }]),
            io_queue_recursive: None,
            io_service_time_recursive: None,
            io_wait_time_recursive: None,
            io_merged_recursive: None,
            io_time_recursive: None,
            sectors_recursive: Some(vec![]),
        };

        let expected = vec![
            ContainerBlkio {
                name: "io_service_bytes_recursive",
                major: 8,
                minor: 0,
                op: "read".to_string(),
                value: 1024,
            },
            ContainerBlkio {
                name: "io_service_bytes_recursive",
                major: 8,
                minor: 0,
                op: "write".to_string(),
                value: 2048,
            },
            ContainerBlkio {
                name: "io_serviced_recursive",
                major: 252,
                minor: 1,
                op: "total".to_string(),
                value: 5,
            },
        ];

        let result = ContainerBlkio::from_stats(bollard_stats);

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn should_send_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        let stats = ContainerBlkio {
            name: "io_serviced_recursive",
            major: 8,
            minor: 16,
            op: "read".to_string(),
            value: 12345,
        };

        let expected_payload = AstarteObject::from_iter(
            [
                ("name", AstarteData::String("io_serviced_recursive".into())),
                ("major", AstarteData::LongInteger(8)),
                ("minor", AstarteData::LongInteger(16)),
                ("op", AstarteData::String("read".into())),
                ("value", AstarteData::LongInteger(12345)),
            ]
            .map(|(name, value)| (name.to_string(), value)),
        );

        client
            .expect_send_object_with_timestamp()
            .once()
            .with(
                predicate::eq(ContainerBlkio::INTERFACE),
                predicate::eq(format!("/{id}")),
                predicate::eq(expected_payload),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        stats.send(&id, &mut client, &timestamp).await;
    }
}
