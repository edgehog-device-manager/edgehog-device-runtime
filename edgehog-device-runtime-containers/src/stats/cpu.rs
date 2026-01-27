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

//! Container cpu statistics

use astarte_device_sdk::IntoAstarteObject;
use bollard::secret::ContainerCpuStats;

use super::{IntoAstarteExt, Metric};

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerCpu {
    cpu_usage_total_usage: i64,
    cpu_usage_percpu_usage: Vec<i64>,
    cpu_usage_usage_in_kernelmode: i64,
    cpu_usage_usage_in_usermode: i64,
    system_cpu_usage: i64,
    online_cpus: i32,
    throttling_data_periods: i64,
    throttling_data_throttled_periods: i64,
    throttling_data_throttled_time: i64,
    pre_cpu_usage_total_usage: i64,
    pre_cpu_usage_percpu_usage: Vec<i64>,
    pre_cpu_usage_usage_in_kernelmode: i64,
    pre_cpu_usage_usage_in_usermode: i64,
    pre_system_cpu_usage: i64,
    pre_online_cpus: i32,
    pre_throttling_data_periods: i64,
    pre_throttling_data_throttled_periods: i64,
    pre_throttling_data_throttled_time: i64,
}

impl ContainerCpu {
    pub(crate) fn from_stats(cpu: ContainerCpuStats, pre: ContainerCpuStats) -> Self {
        let usage = cpu.cpu_usage.unwrap_or_default();
        let throttling = cpu.throttling_data.unwrap_or_default();

        let pre_usage = pre.cpu_usage.unwrap_or_default();
        let pre_throttling = pre.throttling_data.unwrap_or_default();

        Self {
            cpu_usage_total_usage: usage.total_usage.into_astarte(),
            cpu_usage_percpu_usage: usage.percpu_usage.into_astarte(),
            cpu_usage_usage_in_kernelmode: usage.usage_in_kernelmode.into_astarte(),
            cpu_usage_usage_in_usermode: usage.usage_in_usermode.into_astarte(),
            system_cpu_usage: cpu.system_cpu_usage.into_astarte(),
            online_cpus: cpu.online_cpus.into_astarte(),
            throttling_data_periods: throttling.periods.into_astarte(),
            throttling_data_throttled_periods: throttling.throttled_periods.into_astarte(),
            throttling_data_throttled_time: throttling.throttled_time.into_astarte(),
            pre_cpu_usage_total_usage: pre_usage.total_usage.into_astarte(),
            pre_cpu_usage_percpu_usage: pre_usage.percpu_usage.into_astarte(),
            pre_cpu_usage_usage_in_kernelmode: pre_usage.usage_in_kernelmode.into_astarte(),
            pre_cpu_usage_usage_in_usermode: pre_usage.usage_in_usermode.into_astarte(),
            pre_system_cpu_usage: pre.system_cpu_usage.into_astarte(),
            pre_online_cpus: pre.online_cpus.into_astarte(),
            pre_throttling_data_periods: pre_throttling.periods.into_astarte(),
            pre_throttling_data_throttled_periods: pre_throttling.throttled_periods.into_astarte(),
            pre_throttling_data_throttled_time: pre_throttling.throttled_time.into_astarte(),
        }
    }
}

impl Metric for ContainerCpu {
    const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.stats.ContainerCpu";
    const METRIC_NAME: &'static str = "container cpu";
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use bollard::models::{ContainerCpuUsage, ContainerThrottlingData};
    use mockall::{Sequence, predicate};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    // Helper function to create mock bollard::models::CpuStats to reduce boilerplate
    #[allow(clippy::too_many_arguments)]
    fn create_mock_cpu_stats(
        total: u64,
        percpu: Vec<u64>,
        kernel: u64,
        user: u64,
        system: u64,
        online: u32,
        periods: u64,
        throttled_periods: u64,
        throttled_time: u64,
    ) -> ContainerCpuStats {
        ContainerCpuStats {
            cpu_usage: Some(ContainerCpuUsage {
                total_usage: Some(total),
                percpu_usage: Some(percpu),
                usage_in_kernelmode: Some(kernel),
                usage_in_usermode: Some(user),
            }),
            system_cpu_usage: Some(system),
            online_cpus: Some(online),
            throttling_data: Some(ContainerThrottlingData {
                periods: Some(periods),
                throttled_periods: Some(throttled_periods),
                throttled_time: Some(throttled_time),
            }),
        }
    }

    #[test]
    fn from_container_stats_should_correctly_convert() {
        let cpu_stats = create_mock_cpu_stats(1000, vec![200, 800], 400, 600, 5000, 4, 10, 1, 100);
        let precpu_stats = create_mock_cpu_stats(500, vec![100, 400], 200, 300, 2500, 2, 5, 0, 0);

        let result = ContainerCpu::from_stats(cpu_stats, precpu_stats);

        let expected = ContainerCpu {
            cpu_usage_total_usage: 1000,
            cpu_usage_percpu_usage: vec![200, 800],
            cpu_usage_usage_in_kernelmode: 400,
            cpu_usage_usage_in_usermode: 600,
            system_cpu_usage: 5000,
            online_cpus: 4,
            throttling_data_periods: 10,
            throttling_data_throttled_periods: 1,
            throttling_data_throttled_time: 100,
            pre_cpu_usage_total_usage: 500,
            pre_cpu_usage_percpu_usage: vec![100, 400],
            pre_cpu_usage_usage_in_kernelmode: 200,
            pre_cpu_usage_usage_in_usermode: 300,
            pre_system_cpu_usage: 2500,
            pre_online_cpus: 2,
            pre_throttling_data_periods: 5,
            pre_throttling_data_throttled_periods: 0,
            pre_throttling_data_throttled_time: 0,
        };

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn should_send_stats() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();
        let timestamp = Utc::now();

        let stats = ContainerCpu {
            cpu_usage_total_usage: 1000,
            cpu_usage_percpu_usage: vec![200, 800],
            cpu_usage_usage_in_kernelmode: 400,
            cpu_usage_usage_in_usermode: 600,
            system_cpu_usage: 5000,
            online_cpus: 4,
            throttling_data_periods: 10,
            throttling_data_throttled_periods: 1,
            throttling_data_throttled_time: 100,
            pre_cpu_usage_total_usage: 500,
            pre_cpu_usage_percpu_usage: vec![100, 400],
            pre_cpu_usage_usage_in_kernelmode: 200,
            pre_cpu_usage_usage_in_usermode: 300,
            pre_system_cpu_usage: 2500,
            pre_online_cpus: 2,
            pre_throttling_data_periods: 5,
            pre_throttling_data_throttled_periods: 0,
            pre_throttling_data_throttled_time: 0,
        };

        let expected_object = AstarteObject::from_iter(
            [
                ("cpuUsageTotalUsage", AstarteData::LongInteger(1000)),
                (
                    "cpuUsagePercpuUsage",
                    AstarteData::LongIntegerArray(vec![200, 800]),
                ),
                ("cpuUsageUsageInKernelmode", AstarteData::LongInteger(400)),
                ("cpuUsageUsageInUsermode", AstarteData::LongInteger(600)),
                ("systemCpuUsage", AstarteData::LongInteger(5000)),
                ("onlineCpus", AstarteData::Integer(4)),
                ("throttlingDataPeriods", AstarteData::LongInteger(10)),
                (
                    "throttlingDataThrottledPeriods",
                    AstarteData::LongInteger(1),
                ),
                ("throttlingDataThrottledTime", AstarteData::LongInteger(100)),
                ("preCpuUsageTotalUsage", AstarteData::LongInteger(500)),
                (
                    "preCpuUsagePercpuUsage",
                    AstarteData::LongIntegerArray(vec![100, 400]),
                ),
                (
                    "preCpuUsageUsageInKernelmode",
                    AstarteData::LongInteger(200),
                ),
                ("preCpuUsageUsageInUsermode", AstarteData::LongInteger(300)),
                ("preSystemCpuUsage", AstarteData::LongInteger(2500)),
                ("preOnlineCpus", AstarteData::Integer(2)),
                ("preThrottlingDataPeriods", AstarteData::LongInteger(5)),
                (
                    "preThrottlingDataThrottledPeriods",
                    AstarteData::LongInteger(0),
                ),
                (
                    "preThrottlingDataThrottledTime",
                    AstarteData::LongInteger(0),
                ),
            ]
            .map(|(k, v)| (k.to_string(), v)),
        );

        client
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.stats.ContainerCpu"),
                predicate::eq(format!("/{id}")),
                predicate::eq(expected_object),
                predicate::eq(timestamp),
            )
            .returning(|_, _, _, _| Ok(()));

        stats.send(&id, &mut client, &timestamp).await;
    }
}
