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

use std::{collections::HashMap, num::ParseIntError};

use astarte_device_sdk::{event::FromEventError, DeviceEvent, FromEvent};
use container::CreateContainer;
use deployment::{CreateDeployment, DeploymentCommand, DeploymentUpdate};

use crate::store::container::RestartPolicyError;

use self::{image::CreateImage, network::CreateNetwork, volume::CreateVolume};

pub mod container;
pub mod deployment;
pub mod image;
pub mod network;
pub mod volume;

/// Error from handling the Astarte request.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ReqError {
    /// couldn't parse option, expected key=value but got {0}
    Option(String),
    /// couldn't parse container restart policy
    RestartPolicy(#[from] RestartPolicyError),
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
pub enum ContainerRequest {
    /// Request to create an image.
    Image(CreateImage),
    /// Request to create a volume.
    Volume(CreateVolume),
    /// Request to create a network.
    Network(CreateNetwork),
    /// Request to create a container.
    Container(CreateContainer),
    /// Request to create a deployment.
    Deployment(CreateDeployment),
    /// Command for a deployment
    DeploymentCommand(DeploymentCommand),
    /// Update between two deployments
    DeploymentUpdate(DeploymentUpdate),
}

impl FromEvent for ContainerRequest {
    type Err = FromEventError;

    fn from_event(value: DeviceEvent) -> Result<Self, Self::Err> {
        match value.interface.as_str() {
            "io.edgehog.devicemanager.apps.CreateImageRequest" => {
                CreateImage::from_event(value).map(ContainerRequest::Image)
            }
            "io.edgehog.devicemanager.apps.CreateVolumeRequest" => {
                CreateVolume::from_event(value).map(ContainerRequest::Volume)
            }
            "io.edgehog.devicemanager.apps.CreateNetworkRequest" => {
                CreateNetwork::from_event(value).map(ContainerRequest::Network)
            }
            "io.edgehog.devicemanager.apps.CreateContainerRequest" => {
                CreateContainer::from_event(value).map(ContainerRequest::Container)
            }
            "io.edgehog.devicemanager.apps.CreateDeploymentRequest" => {
                CreateDeployment::from_event(value).map(ContainerRequest::Deployment)
            }
            "io.edgehog.devicemanager.apps.DeploymentCommand" => {
                DeploymentCommand::from_event(value).map(ContainerRequest::DeploymentCommand)
            }
            "io.edgehog.devicemanager.apps.DeploymentUpdate" => {
                DeploymentUpdate::from_event(value).map(ContainerRequest::DeploymentUpdate)
            }
            _ => Err(FromEventError::Interface(value.interface.clone())),
        }
    }
}

/// Split a key=value slice into an [`HashMap`].
fn parse_kv_map<S>(input: &[S]) -> Result<HashMap<String, String>, ReqError>
where
    S: AsRef<str>,
{
    input
        .iter()
        .map(|k_v| {
            k_v.as_ref()
                .split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| ReqError::Option(k_v.as_ref().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use astarte_device_sdk::Value;
    use network::{tests::create_network_request_event, CreateNetwork};

    use crate::requests::{image::CreateImage, ContainerRequest};

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

        let request = ContainerRequest::from_event(event).unwrap();

        let expect = ContainerRequest::Image(CreateImage {
            id: "id".to_string(),
            reference: "reference".to_string(),
            registry_auth: "registry_auth".to_string(),
        });

        assert_eq!(request, expect);
    }

    #[test]
    fn should_parse_kv_map() {
        let values = ["foo=bar", "some="];

        let map = parse_kv_map(&values).unwrap();

        assert_eq!(map.len(), 2);
        assert_eq!(map.get("foo").unwrap(), "bar");
        assert_eq!(map.get("some").unwrap(), "");

        let invalid = ["nope"];

        let err = parse_kv_map(&invalid).unwrap_err();

        assert!(matches!(err, ReqError::Option(opt) if opt == "nope"))
    }

    #[test]
    fn from_event_network() {
        let event = create_network_request_event("id", "driver", &[]);

        let request = ContainerRequest::from_event(event).unwrap();

        let expect = CreateNetwork {
            id: "id".to_string(),
            driver: "driver".to_string(),
            internal: false,
            enable_ipv6: false,
            options: Vec::new(),
        };

        assert_eq!(request, ContainerRequest::Network(expect));
    }
}
