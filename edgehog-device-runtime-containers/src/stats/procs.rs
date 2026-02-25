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

//! Container and processes stats.

use astarte_device_sdk::IntoAstarteObject;

use crate::stats::IntoAstarteExt;

use super::Metric;

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerProcesses {
    current: i64,
    limit: i64,
}

impl Metric for ContainerProcesses {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerProcesses";
    const METRIC_NAME: &'static str = "container processes";
}

impl From<bollard::models::ContainerPidsStats> for ContainerProcesses {
    fn from(value: bollard::models::ContainerPidsStats) -> Self {
        Self {
            current: value.current.into_astarte(),
            limit: value.limit.into_astarte(),
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{
        AstarteData, aggregate::AstarteObject, store::SqliteStore, transport::mqtt::Mqtt,
    };
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn should_convert_from_bollard() {
        let simple = bollard::models::ContainerPidsStats {
            current: Some(42),
            limit: Some(120),
        };

        let exp_simple = ContainerProcesses {
            current: 42,
            limit: 120,
        };

        let res = ContainerProcesses::from(simple);
        assert_eq!(res, exp_simple);

        let empty = bollard::models::ContainerPidsStats {
            current: None,
            limit: None,
        };

        let exp_empty = ContainerProcesses {
            current: 0,
            limit: 0,
        };

        let result_none = ContainerProcesses::from(empty);
        assert_eq!(result_none, exp_empty);
    }

    #[tokio::test]
    async fn should_send_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        let stat = ContainerProcesses {
            current: 42,
            limit: 120,
        };

        let expected_payload = AstarteObject::from_iter(
            [
                ("current", AstarteData::LongInteger(42)),
                ("limit", AstarteData::LongInteger(120)),
            ]
            .map(|(name, value)| (name.to_string(), value)),
        );

        client
            .expect_send_object_with_timestamp()
            .once()
            .with(
                predicate::eq(ContainerProcesses::INTERFACE),
                predicate::eq(format!("/{id}")),
                predicate::eq(expected_payload),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        stat.send(&id, &mut client, &timestamp).await;
    }
}
