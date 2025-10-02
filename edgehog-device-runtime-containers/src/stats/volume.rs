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

use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::IntoAstarteObject;

use super::Metric;

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct VolumeUsage {
    driver: String,
    mountpoint: String,
    created_at: DateTime<Utc>,
    usage_data_size: i64,
    usage_data_ref_count: i64,
}

impl Metric for VolumeUsage {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.VolumeUsage";
    const METRIC_NAME: &'static str = "volume usage";
}

impl From<bollard::models::Volume> for VolumeUsage {
    fn from(value: bollard::models::Volume) -> Self {
        let usage_data = value.usage_data.unwrap_or_default();
        Self {
            driver: value.driver,
            mountpoint: value.mountpoint,
            created_at: value.created_at.unwrap_or(DateTime::UNIX_EPOCH),
            usage_data_size: usage_data.size,
            usage_data_ref_count: usage_data.ref_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{
        aggregate::AstarteObject, chrono::TimeZone, store::SqliteStore, transport::mqtt::Mqtt,
        AstarteData,
    };
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn should_convert_from_bollard() {
        let created_at_time = Utc.with_ymd_and_hms(2025, 8, 27, 12, 0, 0).unwrap();
        let bollard_volume_full = bollard::models::Volume {
            driver: "local".to_string(),
            mountpoint: "/var/lib/docker/volumes/my-vol/_data".to_string(),
            created_at: Some(created_at_time),
            usage_data: Some(bollard::models::VolumeUsageData {
                size: 4096,
                ref_count: 3,
            }),
            ..Default::default()
        };

        let expected_volume_usage_full = VolumeUsage {
            driver: "local".to_string(),
            mountpoint: "/var/lib/docker/volumes/my-vol/_data".to_string(),
            created_at: created_at_time,
            usage_data_size: 4096,
            usage_data_ref_count: 3,
        };

        let result_full = VolumeUsage::from(bollard_volume_full);
        assert_eq!(result_full, expected_volume_usage_full);

        let bollard_volume_none = bollard::models::Volume {
            driver: "overlay2".to_string(),
            mountpoint: "/var/lib/docker/volumes/another-vol/_data".to_string(),
            created_at: None,
            usage_data: None,
            ..Default::default()
        };

        let expected_volume_usage_none = VolumeUsage {
            driver: "overlay2".to_string(),
            mountpoint: "/var/lib/docker/volumes/another-vol/_data".to_string(),
            created_at: DateTime::UNIX_EPOCH,
            usage_data_size: 0,
            usage_data_ref_count: 0,
        };

        let result_none = VolumeUsage::from(bollard_volume_none);
        assert_eq!(result_none, expected_volume_usage_none);
    }

    #[tokio::test]
    async fn should_send_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let id = Uuid::new_v4();
        let timestamp = Utc::now();
        let created_at_time = Utc.with_ymd_and_hms(2025, 8, 27, 12, 30, 0).unwrap();

        let volume_stat = VolumeUsage {
            driver: "local".to_string(),
            mountpoint: "/data/my-volume".to_string(),
            created_at: created_at_time,
            usage_data_size: 8192,
            usage_data_ref_count: 5,
        };

        let expected_payload = AstarteObject::from_iter(
            [
                ("driver", AstarteData::String("local".into())),
                ("mountpoint", AstarteData::String("/data/my-volume".into())),
                ("createdAt", AstarteData::DateTime(created_at_time)),
                ("usageDataSize", AstarteData::LongInteger(8192)),
                ("usageDataRefCount", AstarteData::LongInteger(5)),
            ]
            .map(|(name, value)| (name.to_string(), value)),
        );

        client
            .expect_send_object_with_timestamp()
            .once()
            .with(
                predicate::eq(VolumeUsage::INTERFACE),
                predicate::eq(format!("/{}", id)),
                predicate::eq(expected_payload),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        volume_stat.send(&id, &mut client, &timestamp).await;
    }
}
