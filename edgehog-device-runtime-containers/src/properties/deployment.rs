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
use uuid::Uuid;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableDeployments";

/// Available deployment property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableDeployment<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableDeployment<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

#[async_trait]
impl AvailableProp for AvailableDeployment<'_> {
    type Data = DeploymentStatus;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn field() -> &'static str {
        "status"
    }

    fn id(&self) -> &Uuid {
        self.id
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
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_store_deployment() {
        let id = Uuid::new_v4();

        let deployment = AvailableDeployment { id: &id };

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

        deployment
            .send(&client, DeploymentStatus::Stopped)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn should_unset_deployment() {
        let id = Uuid::new_v4();

        let deployment = AvailableDeployment { id: &id };

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path| {
                interface == "io.edgehog.devicemanager.apps.AvailableDeployments"
                    && path == format!("/{id}/status")
            })
            .returning(|_, _| Ok(()));

        deployment.unset(&client).await.unwrap();
    }
}
