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

//! Configure the Edgehog Device Forwarder.

use serde::{Deserialize, Serialize};

use crate::Mergiable;

/// Configuration for the forwarder service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ForwarderConfig {
    /// Flag to enable the forwarder
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl Mergiable for ForwarderConfig {
    fn merge(&mut self, value: Self) {
        let Self { enabled } = value;

        self.enabled = enabled.or(self.enabled);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    pub(crate) fn mock_forwarder_config() -> Option<ForwarderConfig> {
        Some(ForwarderConfig {
            enabled: Some(true),
        })
    }

    #[test]
    fn forwarder_config_roundtrip() {
        let exp = mock_forwarder_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ForwarderConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn forwarder_config_empty_roundtrip() {
        let exp = ForwarderConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ForwarderConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn forwarder_config_disabled_roundtrip() {
        let exp = ForwarderConfig {
            enabled: Some(false),
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ForwarderConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn forwarder_config_merge() {
        let mut base = ForwarderConfig { enabled: None };
        let override_cfg = ForwarderConfig {
            enabled: Some(true),
        };
        let exp = ForwarderConfig {
            enabled: Some(true),
        };

        base.merge(override_cfg);
        assert_eq!(base, exp);

        let override_none = ForwarderConfig { enabled: None };
        base.merge(override_none);
        assert_eq!(base, exp);
    }
}
