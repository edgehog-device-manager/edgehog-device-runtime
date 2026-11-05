// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

use edgehog_config::ota::{OtaConfig, RaucDbus, Reboot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtaArgs {
    pub enabled: bool,
    pub reboot: Reboot,
    pub streaming: bool,
    /// RAUC configuration for the OTA
    pub rauc: RaucConfig,
}

impl Default for OtaArgs {
    fn default() -> Self {
        Self {
            enabled: true,
            reboot: Default::default(),
            streaming: Default::default(),
            rauc: Default::default(),
        }
    }
}

impl From<Option<OtaConfig>> for OtaArgs {
    fn from(value: Option<OtaConfig>) -> Self {
        let Some(OtaConfig {
            enabled,
            reboot,
            streaming,
            rauc,
        }) = value
        else {
            return Self::default();
        };

        Self {
            enabled: enabled.unwrap_or(true),
            reboot: reboot.unwrap_or_default(),
            streaming: streaming.unwrap_or_default(),
            rauc: RaucConfig::from(rauc),
        }
    }
}

/// Configuration for RAUC
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RaucConfig {
    /// DBUS socket to connect to
    pub dbus_socket: RaucDbus,
}

impl From<Option<edgehog_config::ota::RaucConfig>> for RaucConfig {
    fn from(value: Option<edgehog_config::ota::RaucConfig>) -> Self {
        let Some(edgehog_config::ota::RaucConfig { dbus_socket }) = value else {
            return Self::default();
        };

        Self {
            dbus_socket: dbus_socket.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn default_values() {
        let exp = OtaArgs {
            enabled: true,
            reboot: Reboot::Default,
            streaming: false,
            rauc: RaucConfig {
                dbus_socket: RaucDbus::System,
            },
        };

        assert_eq!(OtaArgs::default(), exp);
    }
}
