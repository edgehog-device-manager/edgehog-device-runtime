// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

//! Configuration for the service

use edgehog_config::service::{Listener, ServiceConfig};

/// Configuration for the [`EdgehogService`](crate::service::EdgehogService)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServiceArgs {
    /// Flag to enable the service
    pub enabled: bool,
    /// Listener for the service
    pub listener: Listener,
}

impl From<Option<ServiceConfig>> for ServiceArgs {
    fn from(value: Option<ServiceConfig>) -> Self {
        let Some(ServiceConfig { enabled, listener }) = value else {
            return Self::default();
        };

        Self {
            enabled: enabled.unwrap_or(true),
            listener: listener.unwrap_or_default(),
        }
    }
}
