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

use astarte_device_sdk::AstarteData;
use uuid::Uuid;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableContainers";

/// Available
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableContainer<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableContainer<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

impl AvailableProp for AvailableContainer<'_> {
    type Data = ContainerStatus;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id
    }

    fn field() -> &'static str {
        "status"
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
    Running,
    /// The [`Container`](crate::container::Container) was stopped or crashed.
    Stopped,
}

impl Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Received => write!(f, "Received"),
            ContainerStatus::Created => write!(f, "Created"),
            ContainerStatus::Running => write!(f, "Running"),
            ContainerStatus::Stopped => write!(f, "Stopped"),
        }
    }
}

impl From<ContainerStatus> for AstarteData {
    fn from(value: ContainerStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_store_container() {
        let id = Uuid::new_v4();

        let statuses = [
            ContainerStatus::Received,
            ContainerStatus::Created,
            ContainerStatus::Running,
            ContainerStatus::Stopped,
        ];
        for status in statuses {
            let container = AvailableContainer::new(&id);

            let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
            let mut seq = Sequence::new();

            client
                .expect_set_property()
                .once()
                .in_sequence(&mut seq)
                .withf(move |interface, path, value| {
                    interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                        && path == format!("/{id}/status")
                        && *value == status.to_string()
                })
                .returning(|_, _, _| Ok(()));

            container.send(&mut client, status).await.unwrap();
        }
    }

    #[tokio::test]
    async fn should_unset_container() {
        let id = Uuid::new_v4();

        let container = AvailableContainer::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableContainers"),
                predicate::eq(format!("/{id}/status")),
            )
            .returning(|_, _| Ok(()));

        container.unset(&mut client).await.unwrap();
    }
}
