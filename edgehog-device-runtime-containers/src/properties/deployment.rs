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

//! Available [`Image`](crate::docker::image::Image) property.

use std::fmt::Display;

use astarte_device_sdk::AstarteType;
use async_trait::async_trait;

use super::{AvailableProp, Client, PropError};

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableDeployments";

/// Available deployment property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableDeployments<S> {
    id: S,
    status: DeploymentStatus,
}

impl<S> AvailableDeployments<S> {
    pub(crate) fn new(id: S, status: DeploymentStatus) -> Self {
        Self { id, status }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableDeployments<S>
where
    S: AsRef<str> + Sync,
{
    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }

    async fn send<D>(&self, device: &D) -> Result<(), PropError>
    where
        D: Client + Sync + 'static,
    {
        self.send_field(device, "status", self.status).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeploymentStatus {
    Stopped,
    Started,
}

impl Display for DeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeploymentStatus::Started => write!(f, "Started"),
            DeploymentStatus::Stopped => write!(f, "Stopped"),
        }
    }
}

impl From<DeploymentStatus> for AstarteType {
    fn from(value: DeploymentStatus) -> Self {
        AstarteType::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk_mock::{mockall::Sequence, MockDeviceClient};

    use super::*;

    #[tokio::test]
    async fn should_store_image() {
        let id = "ID";

        let deployment = AvailableDeployments {
            id,
            status: DeploymentStatus::Stopped,
        };

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_send()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, status: &DeploymentStatus| {
                interface == "io.edgehog.devicemanager.apps.AvailableDeployments"
                    && path == format!("/{id}/status")
                    && *status == DeploymentStatus::Stopped
            })
            .returning(|_, _, _| Ok(()));

        deployment.send(&client).await.unwrap();
    }
}
