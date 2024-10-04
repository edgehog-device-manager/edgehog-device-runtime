// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Container requests sent from Astarte.

use astarte_device_sdk::{event::FromEventError, DeviceEvent, FromEvent};

use crate::properties::BindingError;

/// Error from handling the Astarte request.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ReqError {
    /// couldn't parse option, expected key=value but got {0}
    Option(String),
    /// couldn't parse container restart policy: {0}
    RestartPolicy(String),
    /// couldn't parse port binding
    PortBinding(#[from] BindingError),
}

/// Create request from Astarte.
#[derive(Debug, Clone, PartialEq)]
pub enum CreateRequests {}

impl FromEvent for CreateRequests {
    type Err = FromEventError;

    fn from_event(value: DeviceEvent) -> Result<Self, Self::Err> {
        #[allow(clippy::match_single_binding)]
        match value.interface.as_str() {
            _ => Err(FromEventError::Interface(value.interface.clone())),
        }
    }
}
