// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
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

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableDeviceMappings";

/// Available device_mapping property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableDeviceMapping<'a> {
    id: &'a Uuid,
}

impl<'a> AvailableDeviceMapping<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self { id }
    }
}

#[async_trait]
impl AvailableProp for AvailableDeviceMapping<'_> {
    type Data = bool;

    fn interface() -> &'static str {
        INTERFACE
    }

    fn id(&self) -> &Uuid {
        self.id
    }

    fn field() -> &'static str {
        "present"
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk_mock::{mockall::Sequence, MockDeviceClient};
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_store_device_mapping() {
        let id = Uuid::new_v4();

        let device_mapping = AvailableDeviceMapping::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableDeviceMappings"),
                predicate::eq(format!("/{id}/present")),
                predicate::eq(AstarteData::Boolean(true)),
            )
            .returning(|_, _, _| Ok(()));

        device_mapping.send(&mut client, true).await.unwrap();
    }

    #[tokio::test]
    async fn should_unset_device_mapping() {
        let id = Uuid::new_v4();

        let device_mapping = AvailableDeviceMapping::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableDeviceMappings"),
                predicate::eq(format!("/{id}/present")),
            )
            .returning(|_, _| Ok(()));

        device_mapping.unset(&mut client).await.unwrap();
    }
}
