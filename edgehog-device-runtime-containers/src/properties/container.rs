// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

//! Available [`Container`](crate::docker::container::Container) property.

use std::fmt::Display;

use astarte_device_sdk::{AstarteData, Client};
use bollard::models::ContainerInspectResponse;
use uuid::Uuid;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableContainers";

/// Available
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AvailableContainer<'a> {
    id: &'a Uuid,
    image_id: Option<String>,
    exit_code: Option<i64>,
    health_status: Option<String>,
    health_failed: Option<i64>,
    restart_count: Option<i64>,
}

impl<'a> AvailableContainer<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self {
            id,
            image_id: None,
            exit_code: None,
            health_status: None,
            health_failed: None,
            restart_count: None,
        }
    }

    pub(crate) fn with_inspect(id: &'a Uuid, inspect: ContainerInspectResponse) -> Self {
        let (exit_code, health_status, health_failed) = inspect
            .state
            .map(|state| {
                let (health_status, health_failed) = state
                    .health
                    .map(|health| {
                        (
                            health.status.map(|status| status.to_string()),
                            health.failing_streak,
                        )
                    })
                    .unwrap_or_default();

                (state.exit_code, health_status, health_failed)
            })
            .unwrap_or_default();

        Self {
            id,
            image_id: inspect.image,
            exit_code,
            health_status,
            health_failed,
            restart_count: inspect.restart_count,
        }
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

    async fn send_status<D>(
        self,
        device: &mut D,
        data: ContainerStatus,
    ) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        Self::send_field(self.id, device, "status", data).await?;

        Ok(())
    }

    async fn send<D>(self, device: &mut D, data: Self::Data) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        let Self {
            id,
            image_id,
            exit_code,
            health_status,
            health_failed,
            restart_count,
        } = self;

        Self::send_field(id, device, "status", data).await?;
        Self::send_opt_field(id, device, "imageId", image_id).await?;
        Self::send_opt_field(id, device, "exitCode", exit_code).await?;
        Self::send_opt_field(id, device, "health/status", health_status).await?;
        Self::send_opt_field(id, device, "health/failed", health_failed).await?;
        Self::send_opt_field(id, device, "restartCount", restart_count).await?;

        Ok(())
    }

    async fn unset<D>(&self, device: &mut D) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        let id = self.id;

        Self::unset_field(id, device, "status").await?;
        Self::unset_field(id, device, "imageId").await?;
        Self::unset_field(id, device, "exitCode").await?;
        Self::unset_field(id, device, "health/status").await?;
        Self::unset_field(id, device, "health/failed").await?;
        Self::unset_field(id, device, "restartCount").await?;

        Ok(())
    }
}

/// Enum of the [`Container`](crate::container::Container) status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerStatus {
    /// Received and stored the CreateContainer.
    Received,
    /// Started creating the container.
    Creating,
    /// The container has been created.
    Created,
    /// The container is starting.
    Starting,
    /// The container is running.
    Running,
    /// Started stopping the container
    Stopping,
    /// The container was stopped or crashed.
    Stopped,
    /// Started deleting the container.
    Deleting,
    /// The container is in an error state.
    Error,
}

impl Display for ContainerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerStatus::Received => write!(f, "Received"),
            ContainerStatus::Creating => write!(f, "Creating"),
            ContainerStatus::Created => write!(f, "Created"),
            ContainerStatus::Starting => write!(f, "Starting"),
            ContainerStatus::Running => write!(f, "Running"),
            ContainerStatus::Stopping => write!(f, "Stopping"),
            ContainerStatus::Stopped => write!(f, "Stopped"),
            ContainerStatus::Deleting => write!(f, "Deleting"),
            ContainerStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<ContainerStatus> for AstarteData {
    fn from(value: ContainerStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};
    use astarte_test_utils::with_insta;
    use rstest::{Context, rstest};
    use uuid::Uuid;

    use crate::properties::tests::{expect_send_field, expect_send_opt_field, expect_unset_field};

    use super::*;

    pub(crate) fn expect_available_container_send(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        container: &AvailableContainer<'_>,
        status: ContainerStatus,
    ) {
        let AvailableContainer {
            id,
            image_id,
            exit_code,
            health_status,
            health_failed,
            restart_count,
        } = container;

        expect_send_field(client, seq, INTERFACE, id, "status", status);
        expect_send_opt_field(client, seq, INTERFACE, id, "imageId", image_id.clone());
        expect_send_opt_field(client, seq, INTERFACE, id, "exitCode", *exit_code);
        expect_send_opt_field(
            client,
            seq,
            INTERFACE,
            id,
            "health/status",
            health_status.clone(),
        );
        expect_send_opt_field(client, seq, INTERFACE, id, "health/failed", *health_failed);
        expect_send_opt_field(client, seq, INTERFACE, id, "restartCount", *restart_count);
    }

    pub(crate) fn expect_available_container_unset(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        id: &Uuid,
    ) {
        expect_unset_field(client, seq, INTERFACE, id, "status");
        expect_unset_field(client, seq, INTERFACE, id, "imageId");
        expect_unset_field(client, seq, INTERFACE, id, "exitCode");
        expect_unset_field(client, seq, INTERFACE, id, "health/status");
        expect_unset_field(client, seq, INTERFACE, id, "health/failed");
        expect_unset_field(client, seq, INTERFACE, id, "restartCount");
    }

    #[rstest]
    #[case(ContainerStatus::Received)]
    #[case(ContainerStatus::Creating)]
    #[case(ContainerStatus::Created)]
    #[case(ContainerStatus::Running)]
    #[case(ContainerStatus::Starting)]
    #[case(ContainerStatus::Stopping)]
    #[case(ContainerStatus::Stopped)]
    #[case(ContainerStatus::Deleting)]
    #[case(ContainerStatus::Error)]
    fn should_convert_status_to_string(#[case] status: ContainerStatus, #[context] ctx: Context) {
        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, status);
        });
    }

    #[tokio::test]
    async fn should_store_container() {
        let id = Uuid::new_v4();

        let statuses = [
            ContainerStatus::Received,
            ContainerStatus::Creating,
            ContainerStatus::Created,
            ContainerStatus::Running,
            ContainerStatus::Starting,
            ContainerStatus::Stopping,
            ContainerStatus::Stopped,
            ContainerStatus::Deleting,
            ContainerStatus::Error,
        ];
        for status in statuses {
            let container = AvailableContainer::new(&id);

            let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
            let mut seq = Sequence::new();

            expect_available_container_send(&mut client, &mut seq, &container, status);

            container.send(&mut client, status).await.unwrap();
        }
    }

    #[tokio::test]
    async fn should_store_container_with_fields() {
        let id = Uuid::new_v4();

        let container = AvailableContainer {
            id: &id,
            image_id: Some("sha256:123456".to_string()),
            exit_code: Some(0),
            health_status: Some("healthy".to_string()),
            health_failed: Some(0),
            restart_count: Some(1),
        };

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        expect_available_container_send(
            &mut client,
            &mut seq,
            &container,
            ContainerStatus::Running,
        );

        container
            .send(&mut client, ContainerStatus::Running)
            .await
            .unwrap();
    }

    #[test]
    fn should_create_with_inspect() {
        let id = Uuid::new_v4();

        let inspect = ContainerInspectResponse {
            image: Some("sha256:123456".to_string()),
            restart_count: Some(1),
            state: Some(bollard::models::ContainerState {
                exit_code: Some(0),
                health: Some(bollard::models::Health {
                    status: Some(bollard::models::HealthStatusEnum::HEALTHY),
                    failing_streak: Some(0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container = AvailableContainer::with_inspect(&id, inspect);

        assert_eq!(
            container,
            AvailableContainer {
                id: &id,
                image_id: Some("sha256:123456".to_string()),
                exit_code: Some(0),
                health_status: Some("healthy".to_string()),
                health_failed: Some(0),
                restart_count: Some(1),
            }
        );
    }

    #[tokio::test]
    async fn should_unset_container() {
        let id = Uuid::new_v4();

        let container = AvailableContainer::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        expect_available_container_unset(&mut client, &mut seq, &id);

        container.unset(&mut client).await.unwrap();
    }
}
