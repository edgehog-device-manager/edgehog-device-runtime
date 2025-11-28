// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Display;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub struct OtaConfig {
    #[serde(default)]
    pub reboot: Reboot,
    #[serde(default)]
    pub streaming: bool,
    /// RAUC configuration for the OTA
    #[serde(default)]
    pub rauc: RaucConfig,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Reboot {
    #[default]
    Default,
    External,
}

/// Configuration for RAUC
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub struct RaucConfig {
    /// DBUS socket to connect to
    #[serde(default)]
    pub dbus_socket: RaucDbus,
}

/// DBUS socket to comunicate with the RAUC service
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
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
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_deserialize() {
        let string = r#"
        reboot = "external"
        streaming = true
        "#;

        let config: OtaConfig = toml::from_str(string).unwrap();

        let exp = OtaConfig {
            reboot: Reboot::External,
            streaming: true,
            rauc: RaucConfig {
                dbus_socket: RaucDbus::System,
            },
        };

        assert_eq!(config, exp);
    }

    #[test]
    fn should_deserialize_default() {
        let string = r#"
        "#;

        let config: OtaConfig = toml::from_str(string).unwrap();

        let exp = OtaConfig {
            reboot: Reboot::Default,
            streaming: false,
            rauc: RaucConfig {
                dbus_socket: RaucDbus::System,
            },
        };

        assert_eq!(config, exp);
    }

    #[test]
    fn should_deserialize_rauc() {
        let string = r#"
        [rauc]
        dbus_socket = "session"
        "#;

        let config: OtaConfig = toml::from_str(string).unwrap();

        let exp = OtaConfig {
            reboot: Reboot::Default,
            streaming: false,
            rauc: RaucConfig {
                dbus_socket: RaucDbus::Session,
            },
        };

        assert_eq!(config, exp);
    }
}
