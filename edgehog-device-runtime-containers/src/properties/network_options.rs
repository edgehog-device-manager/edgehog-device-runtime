// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

use astarte_device_sdk::Client;
use std::fmt::Display;
use uuid::Uuid;

use super::AvailableProp;
use astarte_device_sdk::AstarteData;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableNetworkOptions";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AvailableNetworkOptionsStatus {
    Present,
    #[expect(dead_code)]
    Error,
}

impl Display for AvailableNetworkOptionsStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AvailableNetworkOptionsStatus::Present => write!(f, "Present"),
            AvailableNetworkOptionsStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<AvailableNetworkOptionsStatus> for AstarteData {
    fn from(value: AvailableNetworkOptionsStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

/// Available network_options property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableNetworkOptions<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableNetworkOptions<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

impl AvailableProp for AvailableNetworkOptions<'_> {
    type Data = AvailableNetworkOptionsStatus;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id
    }

    async fn send_status<D>(
        self,
        device: &mut D,
        data: Self::Data,
    ) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        Self::send_field(self.id(), device, "status", data).await
    }

    async fn send<D>(self, device: &mut D, data: Self::Data) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        Self::send_field(self.id(), device, "status", data).await
    }

    async fn unset<D>(&self, device: &mut D) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        Self::unset_field(self.id(), device, "status").await
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_store_network_options() {
        let id = Uuid::new_v4();

        let network_options = AvailableNetworkOptions::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworkOptions"),
                predicate::eq(format!("/{id}/status")),
                predicate::eq(AstarteData::from(AvailableNetworkOptionsStatus::Present)),
            )
            .returning(|_, _, _| Ok(()));

        network_options
            .send(&mut client, AvailableNetworkOptionsStatus::Present)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn should_unset_network_options() {
        let id = Uuid::new_v4();

        let network_options = AvailableNetworkOptions::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworkOptions"),
                predicate::eq(format!("/{id}/status")),
            )
            .returning(|_, _| Ok(()));

        network_options.unset(&mut client).await.unwrap();
    }
}
