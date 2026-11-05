// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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

use serde::Deserialize;
use zbus::OwnedVariant;
use zbus::proxy;

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
pub trait UPower {
    /// Enumerate all power objects on the system.
    fn enumerate_devices(&self) -> zbus::Result<Vec<zbus::OwnedObjectPath>>;

    /// Indicates whether the system is running on battery power. This property is provided for convenience.
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
pub trait Device {
    /// The level of the battery for devices which do not report a percentage but rather a coarse battery level.
    /// If the value is None, then the device does not support coarse battery reporting, and the percentage should be used instead.
    #[zbus(property)]
    fn battery_level(&self) -> zbus::Result<BatteryLevel>;

    ///If the power source is present in the bay. This field is required as some batteries are hot-removable, for example expensive UPS and most laptop batteries.
    //
    // This property is only valid if the property type has the value "battery".
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    /// The amount of energy left in the power source expressed as a percentage between 0 and 100.
    /// Typically this is the same as (energy - energy-empty) / (energy-full - energy-empty).
    /// However, some primitive power sources are capable of only reporting percentages and in this case the energy-* properties will be unset while this property is set.
    //
    // This property is only valid if the property type has the value "battery".
    //
    // The percentage will be an approximation if the battery level is set to something other than None.
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    /// If the power device is used to supply the system. This would be set TRUE for laptop batteries and UPS devices, but set FALSE for wireless mice or PDAs.
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;

    /// Refreshes the data collected from the power source.
    fn refresh(&self) -> zbus::Result<()>;

    ///Unique serial number of the battery.
    #[zbus(property)]
    fn serial(&self) -> zbus::Result<String>;

    /// The battery power state.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<BatteryState>;

    /// If the value is set to "Battery", you will need to verify that the property power-supply has the value "true" before considering it as a laptop battery.
    /// Otherwise it will likely be the battery for a device of an unknown type.
    #[zbus(property, name = "Type")]
    fn power_device_type(&self) -> zbus::Result<PowerDeviceType>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, zbus::Type)]
#[repr(u32)]
pub enum BatteryState {
    Unknown = 0,
    Charging = 1,
    Discharging = 2,
    Empty = 3,
    FullyCharged = 4,
    PendingCharge = 5,
    PendingDischarge = 6,
}

impl TryFrom<u32> for BatteryState {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let this = match value {
            0 => BatteryState::Unknown,
            1 => BatteryState::Charging,
            2 => BatteryState::Discharging,
            3 => BatteryState::Empty,
            4 => BatteryState::FullyCharged,
            5 => BatteryState::PendingCharge,
            6 => BatteryState::PendingDischarge,
            _ => return Err(value),
        };

        Ok(this)
    }
}

impl<'de> Deserialize<'de> for BatteryState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;

        BatteryState::try_from(value).map_err(|error| {
            serde::de::Error::invalid_value(
                serde::de::Unexpected::Unsigned(error.into()),
                &"battery state between 0-6",
            )
        })
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, OwnedVariant, zbus::Type)]
#[repr(u32)]
pub enum PowerDeviceType {
    Unknown = 0,
    LinePower = 1,
    Battery = 2,
    Ups = 3,
    Monitor = 4,
    Mouse = 5,
    Keyboard = 6,
    Pda = 7,
    Phone = 8,
}

impl TryFrom<u32> for PowerDeviceType {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let this = match value {
            0 => PowerDeviceType::Unknown,
            1 => PowerDeviceType::LinePower,
            2 => PowerDeviceType::Battery,
            3 => PowerDeviceType::Ups,
            4 => PowerDeviceType::Monitor,
            5 => PowerDeviceType::Mouse,
            6 => PowerDeviceType::Keyboard,
            7 => PowerDeviceType::Pda,
            8 => PowerDeviceType::Phone,
            _ => return Err(value),
        };

        Ok(this)
    }
}

impl<'de> Deserialize<'de> for PowerDeviceType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;

        PowerDeviceType::try_from(value).map_err(|error| {
            serde::de::Error::invalid_value(
                serde::de::Unexpected::Unsigned(error.into()),
                &"battery state between 0-8",
            )
        })
    }
}

// TODO: decide if needs removing
#[expect(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, zbus::Type)]
#[repr(u32)]
pub enum BatteryLevel {
    Unknown = 0,
    None = 1,
    Low = 3,
    Critical = 4,
    Normal = 5,
    High = 7,
    Full = 8,
}

impl TryFrom<u32> for BatteryLevel {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        let this = match value {
            0 => BatteryLevel::Unknown,
            1 => BatteryLevel::None,
            3 => BatteryLevel::Low,
            4 => BatteryLevel::Critical,
            5 => BatteryLevel::Normal,
            7 => BatteryLevel::High,
            8 => BatteryLevel::Full,
            _ => return Err(value),
        };

        Ok(this)
    }
}

impl<'de> Deserialize<'de> for BatteryLevel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u32::deserialize(deserializer)?;

        BatteryLevel::try_from(value).map_err(|error| {
            serde::de::Error::invalid_value(
                serde::de::Unexpected::Unsigned(error.into()),
                &"battery state between 0, 1, 3, 4, 5, 7,or 8",
            )
        })
    }
}
