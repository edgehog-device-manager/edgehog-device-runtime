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

use std::num::ParseIntError;

use astarte_device_sdk::{event::FromEventError, DeviceEvent, FromEvent};
use image::CreateImage;

pub(crate) mod image;

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

/// Error from parsing a binding
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum BindingError {
    /// couldn't parse {binding} port {value}
    Port {
        /// Binding received
        binding: &'static str,
        /// Port of the binding
        value: String,
        /// Error converting the port
        #[source]
        source: ParseIntError,
    },
}

/// Create request from Astarte.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CreateRequests {
    /// Request to create an [`Image`](crate::image::Image).
    Image(CreateImage),
}

impl FromEvent for CreateRequests {
    type Err = FromEventError;

    fn from_event(value: DeviceEvent) -> Result<Self, Self::Err> {
        match value.interface.as_str() {
            "io.edgehog.devicemanager.apps.CreateImageRequest" => {
                CreateImage::from_event(value).map(CreateRequests::Image)
            }
            _ => Err(FromEventError::Interface(value.interface.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use astarte_device_sdk::Value;

    use crate::requests::{image::CreateImage, CreateRequests};

    #[test]
    fn from_event_image() {
        let fields = [
            ("id", "id"),
            ("reference", "reference"),
            ("registryAuth", "registry_auth"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into()))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateImageRequest".to_string(),
            path: "/image".to_string(),
            data: Value::Object(fields),
        };

        let request = CreateRequests::from_event(event).unwrap();

        let expect = CreateRequests::Image(CreateImage {
            id: "id".to_string(),
            reference: "reference".to_string(),
            registry_auth: "registry_auth".to_string(),
        });

        assert_eq!(request, expect);
    }
}
