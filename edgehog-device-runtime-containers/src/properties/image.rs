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

use super::{AvailableProp, Client};

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableImages";

/// Available
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableImage<'a> {
    id: &'a Id,
    pulled: bool,
}

impl<'a> AvailableImage<'a> {
    pub(crate) fn new(id: &'a Id, pulled: bool) -> Self {
        Self { id, pulled }
    }
}

#[async_trait]
impl AvailableProp for AvailableImage<'_> {
    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id.uuid()
    }

    async fn send<D>(&self, device: &D)
    where
        D: Client + Sync + 'static,
    {
        self.send_field(device, "pulled", self.pulled).await;
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
    async fn should_store_image() {
        let uuid = Uuid::new_v4();
        let id = Id::new(ResourceType::Image, uuid);

        let image = AvailableImage {
            id: &id,
            pulled: true,
        };

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_send()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, pulled: &bool| {
                interface == "io.edgehog.devicemanager.apps.AvailableImages"
                    && path == format!("/{uuid}/pulled")
                    && *pulled
            })
            .returning(|_, _, _| Ok(()));

        image.send(&client).await;
    }
}
