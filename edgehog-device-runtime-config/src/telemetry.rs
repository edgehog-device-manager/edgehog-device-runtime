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

//! Edgehog telemetry configuration

use serde::{Deserialize, Serialize};

use crate::Mergiable;

/// Enables a telemetry interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryInterfaceConfig {
    /// Name of the interface
    pub interface_name: String,
    /// Flag to enable or disable the interface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Period for which the telemetry data is gathered and sent to Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<u64>,
}

impl Mergiable for TelemetryInterfaceConfig {
    fn merge(&mut self, value: Self) {
        let Self {
            interface_name,
            enabled,
            period,
        } = value;

        self.interface_name = interface_name;
        self.enabled = enabled.or(self.enabled);
        self.period = period.or(self.period);
    }
}

impl Mergiable for Vec<TelemetryInterfaceConfig> {
    fn merge(&mut self, value: Self) {
        for item in value {
            if let Some(existing) = self
                .iter_mut()
                .find(|cfg| cfg.interface_name == item.interface_name)
            {
                existing.merge(item);
            } else {
                self.push(item);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    pub(crate) fn mock_telemetry_interface_config() -> Option<TelemetryInterfaceConfig> {
        Some(TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: Some(true),
            period: Some(60),
        })
    }

    #[test]
    fn telemetry_interface_config_roundtrip() {
        let exp = mock_telemetry_interface_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: TelemetryInterfaceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn telemetry_interface_config_empty_roundtrip() {
        let exp = TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: None,
            period: None,
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: TelemetryInterfaceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn telemetry_interface_config_disabled_roundtrip() {
        let exp = TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: Some(false),
            period: None,
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: TelemetryInterfaceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn telemetry_interface_config_merge() {
        let mut base = TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: Some(true),
            period: Some(60),
        };

        let override_cfg = TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: None,
            period: Some(120),
        };

        let exp = TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
            enabled: Some(true),
            period: Some(120),
        };

        base.merge(override_cfg);

        assert_eq!(base, exp);
    }

    #[test]
    fn telemetry_vec_merge() {
        let mut base = vec![
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
                enabled: Some(true),
                period: Some(60),
            },
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.BatteryStatus".to_string(),
                enabled: Some(false),
                period: Some(30),
            },
        ];

        let override_vec = vec![
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
                enabled: Some(false),
                period: None,
            },
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.StorageUsage".to_string(),
                enabled: Some(true),
                period: Some(120),
            },
        ];

        let exp = vec![
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
                enabled: Some(false),
                period: Some(60),
            },
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.BatteryStatus".to_string(),
                enabled: Some(false),
                period: Some(30),
            },
            TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.StorageUsage".to_string(),
                enabled: Some(true),
                period: Some(120),
            },
        ];

        base.merge(override_vec);

        assert_eq!(base, exp);
    }
}
