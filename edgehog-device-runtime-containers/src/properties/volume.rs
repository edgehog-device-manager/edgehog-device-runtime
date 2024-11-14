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

use async_trait::async_trait;
use uuid::Uuid;

use crate::service::Id;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableVolumes";

/// Available volume property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableVolumes<'a> {
    id: &'a Id,
}

impl<'a> AvailableVolumes<'a> {
    pub(crate) fn new(id: &'a Id) -> Self {
        Self { id }
    }
}

#[async_trait]
impl AvailableProp for AvailableVolumes<'_> {
    type Data = bool;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id.uuid()
    }

    fn field() -> &'static str {
        "created"
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
    async fn should_store_volume() {
        let uuid = Uuid::new_v4();
        let id = Id::new(ResourceType::Image, uuid);

        let volume = AvailableVolumes::new(&id);

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_send()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, pulled: &bool| {
                interface == "io.edgehog.devicemanager.apps.AvailableVolumes"
                    && path == format!("/{uuid}/created")
                    && *pulled
            })
            .returning(|_, _, _| Ok(()));

        volume.send(&client, true).await;
    }

    #[tokio::test]
    async fn should_unset_volume() {
        let uuid = Uuid::new_v4();
        let id = Id::new(ResourceType::Image, uuid);

        let volume = AvailableVolumes::new(&id);

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path| {
                interface == "io.edgehog.devicemanager.apps.AvailableVolumes"
                    && path == format!("/{uuid}/created")
            })
            .returning(|_, _| Ok(()));

        volume.unset(&client).await;
    }
}
