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

use super::{AvailableProp, Client, PropError};

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableVolumes";

/// Available volume property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableVolumes<S> {
    id: S,
    created: bool,
}

impl<S> AvailableVolumes<S> {
    pub(crate) fn new(id: S, created: bool) -> Self {
        Self { id, created }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableVolumes<S>
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
        self.send_field(device, "created", self.created).await?;

        Ok(())
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

        let volume = AvailableVolumes { id, created: true };

        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        client
            .expect_send()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, pulled: &bool| {
                interface == "io.edgehog.devicemanager.apps.AvailableVolumes"
                    && path == format!("/{id}/created")
                    && *pulled
            })
            .returning(|_, _, _| Ok(()));

        volume.send(&client).await.unwrap();
    }
}
