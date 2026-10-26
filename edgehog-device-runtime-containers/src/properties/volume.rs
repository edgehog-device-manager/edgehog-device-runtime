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

//! Available [`Image`](crate::docker::image::Image) property.

use std::fmt::Display;

use astarte_device_sdk::AstarteData;
use uuid::Uuid;

use crate::properties::PropertyError;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableVolumes";

/// Available volume property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableVolume<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableVolume<'a> {
    const PRESENT_ENDPOINT: &str = "created";

    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AvailableVolumeStatus {
    Present,
    Missing,
    Error,
    Deleting,
}

impl AvailableVolumeStatus {
    pub(crate) fn is_present(&self) -> bool {
        match self {
            AvailableVolumeStatus::Present => true,
            AvailableVolumeStatus::Missing => false,
            AvailableVolumeStatus::Error => false,
            AvailableVolumeStatus::Deleting => false,
        }
    }
}

impl Display for AvailableVolumeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AvailableVolumeStatus::Present => f.write_str("Present"),
            AvailableVolumeStatus::Missing => f.write_str("Missing"),
            AvailableVolumeStatus::Error => f.write_str("Error"),
            AvailableVolumeStatus::Deleting => f.write_str("Deleting"),
        }
    }
}

impl From<AvailableVolumeStatus> for AstarteData {
    fn from(value: AvailableVolumeStatus) -> Self {
        Self::String(value.to_string())
    }
}

impl AvailableProp for AvailableVolume<'_> {
    type Data = AvailableVolumeStatus;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id
    }

    fn field() -> &'static str {
        "status"
    }

    async fn send<D>(&self, device: &mut D, data: Self::Data) -> Result<(), super::PropertyError>
    where
        D: astarte_device_sdk::prelude::Client + Send + Sync + 'static,
    {
        self.send_field(device, Self::field(), data).await?;
        self.send_field(device, Self::PRESENT_ENDPOINT, data.is_present())
            .await
    }

    async fn unset<D>(&self, device: &mut D) -> Result<(), PropertyError>
    where
        D: astarte_device_sdk::prelude::Client + Send + Sync + 'static,
    {
        self.unset_field(device, Self::field()).await?;
        self.unset_field(device, Self::PRESENT_ENDPOINT).await
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
    async fn should_store_volume() {
        let id = Uuid::new_v4();

        let volume = AvailableVolume::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableVolumes"),
                predicate::eq(format!("/{id}/created")),
                predicate::eq(AstarteData::Boolean(true)),
            )
            .returning(|_, _, _| Ok(()));
        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableVolumes"),
                predicate::eq(format!("/{id}/status")),
                predicate::eq(AstarteData::from(AvailableVolumeStatus::Present)),
            )
            .returning(|_, _, _| Ok(()));

        volume
            .send(&mut client, AvailableVolumeStatus::Present)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn should_unset_volume() {
        let id = Uuid::new_v4();

        let volume = AvailableVolume::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableVolumes"),
                predicate::eq(format!("/{id}/created")),
            )
            .returning(|_, _| Ok(()));

        volume.unset(&mut client).await.unwrap();
    }
}
