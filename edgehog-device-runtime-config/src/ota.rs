// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Configure the Edgehog OTA service

use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// Edgehog OTA configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct OtaConfig {
    /// Enables the OTA service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Configures the reboot behaviour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reboot: Option<Reboot>,
    /// Enables OTA streaming.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    /// RAUC configuration for the OTA
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rauc: Option<RaucConfig>,
}

/// Ota reboot behaviour.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Reboot {
    /// Reboots after installing an OTA update.
    #[default]
    Default,
    /// Doesn't reboot the device.
    ///
    /// The task of rebooting the device is given to an external process.
    External,
}

/// Configuration for RAUC
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RaucConfig {
    /// DBUS socket to connect to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dbus_socket: Option<RaucDbus>,
}

/// DBUS socket to communicate with the RAUC service
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RaucDbus {
    /// Uses the system bus
    #[default]
    System,
    /// Uses the current user session bus
    Session,
}

impl Display for RaucDbus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RaucDbus::System => write!(f, "system"),
            RaucDbus::Session => write!(f, "session"),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;

    use super::*;

    pub(crate) fn mock_ota_config() -> Option<OtaConfig> {
        Some(OtaConfig {
            enabled: Some(true),
            reboot: Some(Reboot::External),
            streaming: Some(true),
            rauc: mock_rauc_config(),
        })
    }

    pub(crate) fn mock_rauc_config() -> Option<RaucConfig> {
        Some(RaucConfig {
            dbus_socket: Some(RaucDbus::System),
        })
    }

    #[test]
    fn ota_config_roundtrip() {
        let exp = mock_ota_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: OtaConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn ota_config_empty_roundtrip() {
        let exp = OtaConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: OtaConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn ota_config_reboot_default_roundtrip() {
        let exp = OtaConfig {
            enabled: Some(false),
            reboot: Some(Reboot::Default),
            streaming: Some(false),
            rauc: Some(RaucConfig {
                dbus_socket: Some(RaucDbus::Session),
            }),
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: OtaConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn rauc_config_roundtrip() {
        let exp = mock_rauc_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: RaucConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn rauc_config_empty_roundtrip() {
        let exp = RaucConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: RaucConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn rauc_config_session_roundtrip() {
        let exp = RaucConfig {
            dbus_socket: Some(RaucDbus::Session),
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: RaucConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn rauc_dbus_display() {
        assert_eq!(RaucDbus::System.to_string(), "system");
        assert_eq!(RaucDbus::Session.to_string(), "session");
    }
}
