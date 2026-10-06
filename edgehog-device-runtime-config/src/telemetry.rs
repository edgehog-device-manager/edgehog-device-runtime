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
