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

//! Available [`Container`](crate::docker::container::Container) property.

use std::fmt::Display;

use astarte_device_sdk::AstarteType;
use async_trait::async_trait;
use uuid::Uuid;

use crate::service::Id;

use super::{AvailableProp, Client, PropError};

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableContainers";

/// Available
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableContainers<'a> {
    id: &'a Id,
    status: ContainerStatus,
}

impl<'a> AvailableContainers<'a> {
    pub(crate) fn new(id: &'a Id, status: ContainerStatus) -> Self {
        Self { id, status }
    }
}

#[async_trait]
impl AvailableProp for AvailableContainers<'_> {
    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id.uuid()
    }

    async fn send<D>(&self, device: &D) -> Result<(), PropError>
    where
        D: Client + Sync + 'static,
    {
        self.send_field(device, "status", self.status).await?;

        Ok(())
    }
}

/// Enum of the container status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerStatus {
    /// Received and stored the CreateContainer.
    Received,
    /// The [`Container`](crate::container::Container) has been created.
    Created,
    /// The [`Container`](crate::container::Container) is running.
    _Running,
    /// The [`Container`](crate::container::Container) was stopped or crashed.
    _Stopped,
}

impl Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Received => write!(f, "Received"),
            ContainerStatus::Created => write!(f, "Created"),
            ContainerStatus::_Running => write!(f, "Running"),
            ContainerStatus::_Stopped => write!(f, "Stopped"),
        }
    }
}

impl From<ContainerStatus> for AstarteType {
    fn from(value: ContainerStatus) -> Self {
        AstarteType::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk_mock::{mockall::Sequence, MockDeviceClient};
    use uuid::Uuid;

    use crate::service::ResourceType;

    use super::*;

    #[tokio::test]
    async fn should_store_container() {
        let uuid = Uuid::new_v4();
        let id = Id::new(ResourceType::Container, uuid);

        let statuses = [
            ContainerStatus::Received,
            ContainerStatus::Created,
            ContainerStatus::_Running,
            ContainerStatus::_Stopped,
        ];
        for status in statuses {
            let container = AvailableContainers { id: &id, status };

            let mut client = MockDeviceClient::<SqliteStore>::new();
            let mut seq = Sequence::new();

            client
                .expect_send()
                .once()
                .in_sequence(&mut seq)
                .withf(move |interface, path, value: &ContainerStatus| {
                    interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                        && path == format!("/{uuid}/status")
                        && *value == status
                })
                .returning(|_, _, _| Ok(()));

            container.send(&client).await.unwrap();
        }
    }
}
