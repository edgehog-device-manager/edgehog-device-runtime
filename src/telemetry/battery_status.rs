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

use astarte_device_sdk::AstarteAggregate;
use std::collections::HashMap;

use crate::error::DeviceManagerError;
use crate::telemetry::upower::device::{BatteryState, DeviceProxy, PowerDeviceType};
use crate::telemetry::upower::UPowerProxy;

#[derive(Debug, AstarteAggregate, PartialEq)]
#[allow(non_snake_case)]
pub struct BatteryStatus {
    levelPercentage: f64,
    levelAbsoluteError: f64,
    /// "Battery status string, any of: Charging, Discharging, Idle, EitherIdleOrCharging, Failure, Removed, Unknown",
    status: String,
}

impl BatteryStatus {
    pub async fn new(level_percentage: f64, device_state: BatteryState, is_present: bool) -> Self {
        let status = get_status(device_state, is_present);
        let level_absolute_error = get_error_level(device_state);

        BatteryStatus {
            levelPercentage: level_percentage,
            levelAbsoluteError: level_absolute_error,
            status,
        }
    }
}

pub async fn get_battery_status() -> Result<HashMap<String, BatteryStatus>, DeviceManagerError> {
    let connection = zbus::Connection::system().await?;
    let upower = UPowerProxy::new(&connection).await?;
    let devices = upower.enumerate_devices().await?;

    let mut result = HashMap::new();
    for device_path in devices {
        dbg!(&result);

        let device = DeviceProxy::builder(&connection)
            .path(device_path)?
            .build()
            .await?;

        if device.power_supply().await?
            && device.power_device_type().await? == PowerDeviceType::Battery
        {
            result.insert(
                device.serial().await?,
                BatteryStatus::new(
                    device.percentage().await?,
                    device.state().await?,
                    device.is_present().await?,
                )
                .await,
            );
        }
    }
    Ok(result)
}

fn get_status(device_state: BatteryState, is_present: bool) -> String {
    match device_state {
        BatteryState::Charging => "Charging".to_string(),
        BatteryState::Discharging => "Discharging".to_string(),
        BatteryState::Unknown => "Unknown".to_string(),
        _ => {
            if is_present {
                "Idle".to_string()
            } else {
                "Removed".to_string()
            }
        }
    }
}

fn get_error_level(device_state: BatteryState) -> f64 {
    if device_state == BatteryState::Unknown {
        100_f64
    } else {
        0_f64
    }
}

#[cfg(test)]
mod tests {
    use crate::telemetry::battery_status::{
        get_battery_status, get_error_level, get_status, BatteryStatus,
    };
    use crate::telemetry::upower::device::BatteryState;

    #[tokio::test]
    async fn battery_unknown_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::Unknown;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present).await;

        assert_eq!(
            battery,
            BatteryStatus {
                levelPercentage: level_percentage,
                levelAbsoluteError: 100.0,
                status: "Unknown".to_string()
            }
        )
    }

    #[tokio::test]
    async fn battery_charging_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::Charging;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present).await;

        assert_eq!(
            battery,
            BatteryStatus {
                levelPercentage: level_percentage,
                levelAbsoluteError: 0.0,
                status: "Charging".to_string()
            }
        )
    }

    #[tokio::test]
    async fn battery_discharging_new_test() {
        let level_percentage: f64 = 50.0;
        let device_state = BatteryState::Discharging;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present).await;

        assert_eq!(
            battery,
            BatteryStatus {
                levelPercentage: level_percentage,
                levelAbsoluteError: 0.0,
                status: "Discharging".to_string()
            }
        )
    }

    #[tokio::test]
    async fn battery_idle_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::FullyCharged;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present).await;

        assert_eq!(
            battery,
            BatteryStatus {
                levelPercentage: level_percentage,
                levelAbsoluteError: 0.0,
                status: "Idle".to_string()
            }
        )
    }

    #[tokio::test]
    async fn battery_removed_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::FullyCharged;
        let is_present = false;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present).await;

        assert_eq!(
            battery,
            BatteryStatus {
                levelPercentage: level_percentage,
                levelAbsoluteError: 0.0,
                status: "Removed".to_string()
            }
        )
    }

    #[tokio::test]
    async fn get_battery_status_test() {
        let battery_status_result = get_battery_status().await;
        println!("{:?}", battery_status_result);
        assert!(battery_status_result.is_ok());
    }

    #[test]
    fn get_status_test() {
        assert_eq!(
            get_status(BatteryState::Charging, true),
            "Charging".to_string()
        );
        assert_eq!(
            get_status(BatteryState::Discharging, true),
            "Discharging".to_string()
        );
        assert_eq!(
            get_status(BatteryState::Unknown, true),
            "Unknown".to_string()
        );
        assert_eq!(
            get_status(BatteryState::FullyCharged, true),
            "Idle".to_string()
        );
        assert_eq!(
            get_status(BatteryState::Empty, false),
            "Removed".to_string()
        );
    }

    #[test]
    fn get_error_level_test() {
        assert_eq!(get_error_level(BatteryState::Charging), 0_f64);
        assert_eq!(get_error_level(BatteryState::Unknown), 100_f64);
    }
}
