// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
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
use tracing::{debug, error};
use zbus::zvariant::OwnedObjectPath;

use crate::data::send_object_with_timestamp;
use crate::telemetry::upower::device::{BatteryState, DeviceProxy, PowerDeviceType};
use crate::telemetry::upower::UPowerProxy;

const INTERFACE: &str = "io.edgehog.devicemanager.BatteryStatus";

#[derive(Debug, IntoAstarteObject, PartialEq)]
#[astarte_object(rename_all = "camelCase")]
pub struct BatteryStatus {
    level_percentage: f64,
    level_absolute_error: f64,
    /// "Battery status string, any of: Charging, Discharging, Idle, EitherIdleOrCharging, Failure, Removed, Unknown",
    status: String,
}

impl BatteryStatus {
    pub fn new(level_percentage: f64, device_state: BatteryState, is_present: bool) -> Self {
        let status = get_status(device_state, is_present);
        let level_absolute_error = get_error_level(device_state);

        BatteryStatus {
            level_percentage,
            level_absolute_error,
            status,
        }
    }

    async fn read(
        connection: &zbus::Connection,
        device_path: &OwnedObjectPath,
    ) -> stable_eyre::Result<Option<(String, Self)>> {
        let device = DeviceProxy::builder(connection)
            .path(device_path)?
            .build()
            .await?;

        if !device.power_supply().await?
            || device.power_device_type().await? != PowerDeviceType::Battery
        {
            debug!("Device {device_path} not a power supply or battery");

            return Ok(None);
        }

        let serial = device
            .serial()
            .await
            .map(|serial| format!("/{}", serial.trim_matches('/')))?;

        let level_percentage = device.percentage().await?;
        let device_state = device.state().await?;
        let is_present = device.is_present().await?;

        Ok(Some((
            serial,
            BatteryStatus::new(level_percentage, device_state, is_present),
        )))
    }
}

pub async fn send_battery_status<C>(client: &mut C)
where
    C: Client,
{
    let connection = match zbus::Connection::system().await {
        Ok(conn) => conn,
        Err(err) => {
            error!(
                "couldn't connect to system dbus: {}",
                stable_eyre::Report::new(err)
            );

            return;
        }
    };

    let devices = match enumerate_devices(&connection).await {
        Ok(devices) => devices,
        Err(err) => {
            error!(
                "couldn't enumerate the device: {}",
                stable_eyre::Report::new(err)
            );

            return;
        }
    };

    for device_path in devices {
        let (battery_slot, status) = match BatteryStatus::read(&connection, &device_path).await {
            Ok(Some(res)) => res,
            Ok(None) => {
                continue;
            }
            Err(err) => {
                error!("couldn't get battery status for {device_path}: {err}");

                continue;
            }
        };

        debug!(battery_slot, ?status, "found battery");

        send_object_with_timestamp(client, INTERFACE, &battery_slot, status, Utc::now()).await;
    }
}

async fn enumerate_devices(connection: &zbus::Connection) -> zbus::Result<Vec<OwnedObjectPath>> {
    let upower = UPowerProxy::new(connection).await?;

    upower.enumerate_devices().await
}

fn get_status(device_state: BatteryState, is_present: bool) -> String {
    match device_state {
        BatteryState::Charging => "Charging".to_string(),
        BatteryState::Discharging => "Discharging".to_string(),
        BatteryState::Unknown => "Unknown".to_string(),
        BatteryState::Empty
        | BatteryState::FullyCharged
        | BatteryState::PendingCharge
        | BatteryState::PendingDischarge => {
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
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;

    use super::*;

    #[test]
    fn battery_unknown_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::Unknown;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present);

        assert_eq!(
            battery,
            BatteryStatus {
                level_percentage,
                level_absolute_error: 100.0,
                status: "Unknown".to_string()
            }
        );
    }

    #[test]
    fn battery_charging_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::Charging;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present);

        assert_eq!(
            battery,
            BatteryStatus {
                level_percentage,
                level_absolute_error: 0.0,
                status: "Charging".to_string()
            }
        );
    }

    #[test]
    fn battery_discharging_new_test() {
        let level_percentage: f64 = 50.0;
        let device_state = BatteryState::Discharging;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present);

        assert_eq!(
            battery,
            BatteryStatus {
                level_percentage,
                level_absolute_error: 0.0,
                status: "Discharging".to_string()
            }
        );
    }

    #[test]
    fn battery_idle_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::FullyCharged;
        let is_present = true;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present);

        assert_eq!(
            battery,
            BatteryStatus {
                level_percentage,
                level_absolute_error: 0.0,
                status: "Idle".to_string()
            }
        );
    }

    #[test]
    fn battery_removed_new_test() {
        let level_percentage: f64 = 100.0;
        let device_state = BatteryState::FullyCharged;
        let is_present = false;

        let battery = BatteryStatus::new(level_percentage, device_state, is_present);

        assert_eq!(
            battery,
            BatteryStatus {
                level_percentage,
                level_absolute_error: 0.0,
                status: "Removed".to_string()
            }
        )
    }

    #[tokio::test]
    async fn get_battery_status_test() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_send_object_with_timestamp()
            .times(..)
            .with(
                predicate::eq("io.edgehog.devicemanager.BatteryStatus"),
                predicate::always(),
                predicate::always(),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        send_battery_status(&mut client).await;
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
