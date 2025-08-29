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

use std::{borrow::Borrow, fmt::Display, num::ParseIntError, ops::Deref};

use astarte_device_sdk::{
    event::FromEventError, types::TypeError, AstarteData, DeviceEvent, FromEvent,
};
use container::{CreateContainer, RestartPolicyError};
use deployment::{CreateDeployment, DeploymentCommand, DeploymentUpdate};
use tracing::error;
use uuid::Uuid;

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
    Container(Box<CreateContainer>),
    /// Request to create a deployment.
    Deployment(CreateDeployment),
    /// Command for a deployment
    DeploymentCommand(DeploymentCommand),
    /// Update between two deployments
    DeploymentUpdate(DeploymentUpdate),
}

impl ContainerRequest {
    pub(crate) fn deployment_id(&self) -> Option<Uuid> {
        match self {
            ContainerRequest::Image(_)
            | ContainerRequest::Volume(_)
            | ContainerRequest::Network(_)
            | ContainerRequest::Container(_) => None,
            ContainerRequest::Deployment(create_deployment) => Some(create_deployment.id.0),
            ContainerRequest::DeploymentCommand(deployment_command) => Some(deployment_command.id),
            ContainerRequest::DeploymentUpdate(deployment_update) => Some(deployment_update.from),
        }
    }
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
                CreateContainer::from_event(value)
                    .map(|value| ContainerRequest::Container(Box::new(value)))
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

/// Wrapper to convert an [`AstarteData`] to [`Uuid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReqUuid(pub(crate) Uuid);

impl Display for ReqUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Borrow<Uuid> for ReqUuid {
    fn borrow(&self) -> &Uuid {
        &self.0
    }
}

impl Deref for ReqUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<&str> for ReqUuid {
    type Error = TypeError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Uuid::parse_str(value).map(ReqUuid).map_err(|err| {
            error!(
                error = format!("{:#}", eyre::Report::new(err)),
                value, "couldn't parse uuid value"
            );

            TypeError::Conversion {
                ctx: format!("couldn't parse uuid value: {value}"),
            }
        })
    }
}

impl TryFrom<AstarteData> for ReqUuid {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        Self::try_from(value.as_str())
    }
}

impl From<ReqUuid> for Uuid {
    fn from(value: ReqUuid) -> Self {
        value.0
    }
}

impl From<&ReqUuid> for Uuid {
    fn from(value: &ReqUuid) -> Self {
        value.0
    }
}

/// Wrapper to convert an [`AstarteData`] to [`Vec<Uuid>`].
///
/// This is required because we cannot implement [`TryFrom<AstarteData>`] for [`Vec<ReqUuid>`], because
/// of the orphan rule.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub(crate) struct VecReqUuid(pub(crate) Vec<ReqUuid>);

impl Deref for VecReqUuid {
    type Target = Vec<ReqUuid>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<AstarteData> for VecReqUuid {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = Vec::<String>::try_from(value)?;

        value
            .iter()
            .map(|v| ReqUuid::try_from(v.as_str()))
            .collect::<Result<Vec<ReqUuid>, TypeError>>()
            .map(VecReqUuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use image::tests::create_image_request_event;
    use network::{tests::create_network_request_event, CreateNetwork};
    use pretty_assertions::assert_eq;

    use crate::requests::ContainerRequest;

    #[test]
    fn from_event_image() {
        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let event = create_image_request_event(id, deployment_id, "reference", "registry_auth");

        let request = ContainerRequest::from_event(event).unwrap();

        let expect = ContainerRequest::Image(CreateImage {
            id: ReqUuid(id),
            deployment_id: ReqUuid(deployment_id),
            reference: "reference".to_string(),
            registry_auth: "registry_auth".to_string(),
        });

        assert_eq!(request, expect);
    }

    #[test]
    fn from_event_network() {
        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let event = create_network_request_event(id, deployment_id, "driver", &[]);

        let request = ContainerRequest::from_event(event).unwrap();

        let expect = CreateNetwork {
            id: ReqUuid(id),
            deployment_id: ReqUuid(deployment_id),
            driver: "driver".to_string(),
            internal: false,
            enable_ipv6: false,
            options: Vec::new(),
        };

        assert_eq!(request, ContainerRequest::Network(expect));
    }
}
