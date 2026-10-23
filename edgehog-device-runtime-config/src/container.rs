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

//! Configure the Edgehog Container service.

use serde::{Deserialize, Serialize};

use crate::Mergiable;

/// Configuration for the container service.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ContainersConfig {
    /// Flag to enable the container service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Flag to make the container service is required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// Maximum number of retries for the initialization of the service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<usize>,
}

impl Mergiable for ContainersConfig {
    fn merge(&mut self, value: Self) {
        let Self {
            enabled,
            required,
            max_retries,
        } = value;

        self.enabled = enabled.or(self.enabled);
        self.required = required.or(self.required);
        self.max_retries = max_retries.or(self.max_retries);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    pub(crate) fn mock_containers_config() -> Option<ContainersConfig> {
        Some(ContainersConfig {
            enabled: Some(true),
            required: Some(true),
            max_retries: Some(3),
        })
    }

    #[test]
    fn containers_config_roundtrip() {
        let exp = mock_containers_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ContainersConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn containers_config_empty_roundtrip() {
        let exp = ContainersConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ContainersConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn containers_config_disabled_roundtrip() {
        let exp = ContainersConfig {
            enabled: Some(false),
            required: None,
            max_retries: None,
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ContainersConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn containers_config_merge() {
        let mut base = ContainersConfig {
            enabled: Some(true),
            required: None,
            max_retries: Some(3),
        };

        let override_cfg = ContainersConfig {
            enabled: None,
            required: Some(false),
            max_retries: Some(5),
        };

        let exp = ContainersConfig {
            enabled: Some(true),
            required: Some(false),
            max_retries: Some(5),
        };

        base.merge(override_cfg);

        assert_eq!(base, exp);
    }
}
