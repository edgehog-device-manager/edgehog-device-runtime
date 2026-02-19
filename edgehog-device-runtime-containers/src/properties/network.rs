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

use uuid::Uuid;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableNetworks";

/// Available network property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableNetwork<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableNetwork<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

impl AvailableProp for AvailableNetwork<'_> {
    type Data = bool;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id
    }

    fn field() -> &'static str {
        "created"
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_store_network() {
        let id = Uuid::new_v4();

        let network = AvailableNetwork::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworks"),
                predicate::eq(format!("/{id}/created")),
                predicate::eq(AstarteData::Boolean(true)),
            )
            .returning(|_, _, _| Ok(()));

        network.send(&mut client, true).await.unwrap();
    }

    #[tokio::test]
    async fn should_unset_network() {
        let id = Uuid::new_v4();

        let network = AvailableNetwork::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworks"),
                predicate::eq(format!("/{id}/created")),
            )
            .returning(|_, _| Ok(()));

        network.unset(&mut client).await.unwrap();
    }
}
