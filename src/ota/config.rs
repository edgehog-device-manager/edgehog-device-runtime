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

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub struct OtaConfig {
    #[serde(default)]
    pub reboot: Reboot,
    #[serde(default)]
    pub streaming: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Reboot {
    #[default]
    Default,
    External,
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
        };

        assert_eq!(config, exp);
    }
}
