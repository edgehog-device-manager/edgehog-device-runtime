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

use astarte_device_sdk::Client;
use bollard::plugin::ImageInspect;
use std::fmt::Display;

use astarte_device_sdk::AstarteData;
use uuid::Uuid;

use super::AvailableProp;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableImages";

/// Available
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableImage<'a> {
    id: &'a Uuid,
    image_id: Option<String>,
    repo_tags: Option<Vec<String>>,
    registry_digest: Option<Vec<String>>,
    size: Option<i64>,
}

impl<'a> AvailableImage<'a> {
    pub(crate) fn new(id: &'a Uuid) -> Self {
        Self {
            id,
            image_id: None,
            repo_tags: None,
            registry_digest: None,
            size: None,
        }
    }

    pub(crate) fn with_inspect(id: &'a Uuid, image_inspect: ImageInspect) -> Self {
        Self {
            id,
            image_id: image_inspect.id,
            repo_tags: image_inspect.repo_tags,
            registry_digest: image_inspect.repo_digests,
            size: image_inspect.size,
        }
    }
}

impl AvailableProp for AvailableImage<'_> {
    type Data = AvailableImageStatus;

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
        Self::send_field(self.id(), device, "pulled", data.is_pulled()).await?;
        Self::send_field(self.id(), device, "status", data).await?;

        Ok(())
    }

    async fn send<D>(self, device: &mut D, data: Self::Data) -> Result<(), super::PropertyError>
    where
        D: astarte_device_sdk::prelude::Client + Send + Sync + 'static,
    {
        let Self {
            id,
            image_id,
            repo_tags,
            registry_digest,
            size,
        } = self;

        Self::send_field(id, device, "pulled", data.is_pulled()).await?;
        Self::send_field(id, device, "status", data).await?;
        Self::send_opt_field(id, device, "imageId", image_id).await?;
        Self::send_opt_field(id, device, "repoTags", repo_tags).await?;
        Self::send_opt_field(id, device, "repoDigests", registry_digest).await?;
        Self::send_opt_field(id, device, "size", size).await?;

        Ok(())
    }

    async fn unset<D>(&self, device: &mut D) -> Result<(), super::PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        Self::unset_field(self.id, device, "pulled").await?;
        Self::unset_field(self.id, device, "status").await?;
        Self::unset_field(self.id, device, "imageId").await?;
        Self::unset_field(self.id, device, "repoTags").await?;
        Self::unset_field(self.id, device, "repoDigests").await?;
        Self::unset_field(self.id, device, "size").await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailableImageStatus {
    Missing,
    Received,
    Pulling,
    Pulled,
    Deleting,
    Error,
}

impl AvailableImageStatus {
    pub fn is_pulled(&self) -> bool {
        match self {
            AvailableImageStatus::Missing
            | AvailableImageStatus::Received
            | AvailableImageStatus::Pulling
            | AvailableImageStatus::Error => false,
            AvailableImageStatus::Pulled | AvailableImageStatus::Deleting => true,
        }
    }
}

impl Display for AvailableImageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AvailableImageStatus::Missing => write!(f, "Missing"),
            AvailableImageStatus::Received => write!(f, "Received"),
            AvailableImageStatus::Pulling => write!(f, "Pulling"),
            AvailableImageStatus::Pulled => write!(f, "Pulled"),
            AvailableImageStatus::Deleting => write!(f, "Deleting"),
            AvailableImageStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<AvailableImageStatus> for AstarteData {
    fn from(status: AvailableImageStatus) -> Self {
        AstarteData::String(status.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::properties::tests::{expect_send_field, expect_send_opt_field, expect_unset_field};
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};

    use rstest::rstest;
    use uuid::Uuid;

    use super::*;

    pub(crate) fn expect_available_image_send(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        image: AvailableImage,
        status: AvailableImageStatus,
    ) {
        let AvailableImage {
            id,
            image_id,
            repo_tags,
            registry_digest,
            size,
        } = image;

        expect_send_field(client, seq, INTERFACE, id, "pulled", status.is_pulled());
        expect_send_field(client, seq, INTERFACE, id, "status", status);
        expect_send_opt_field(client, seq, INTERFACE, id, "imageId", image_id);
        expect_send_opt_field(client, seq, INTERFACE, id, "repoTags", repo_tags);
        expect_send_opt_field(client, seq, INTERFACE, id, "repoDigests", registry_digest);
        expect_send_opt_field(client, seq, INTERFACE, id, "size", size);
    }

    pub(crate) fn expect_status_send(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        image: AvailableImage,
        status: AvailableImageStatus,
    ) {
        expect_send_field(
            client,
            seq,
            INTERFACE,
            image.id,
            "pulled",
            status.is_pulled(),
        );
        expect_send_field(client, seq, INTERFACE, image.id, "status", status);
    }

    pub(crate) fn expect_available_image_unset(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        id: &Uuid,
    ) {
        expect_unset_field(client, seq, INTERFACE, id, "pulled");
        expect_unset_field(client, seq, INTERFACE, id, "status");
        expect_unset_field(client, seq, INTERFACE, id, "imageId");
        expect_unset_field(client, seq, INTERFACE, id, "repoTags");
        expect_unset_field(client, seq, INTERFACE, id, "repoDigests");
        expect_unset_field(client, seq, INTERFACE, id, "size");
    }

    #[rstest]
    #[case(AvailableImageStatus::Missing, AstarteData::String("Missing".to_string()))]
    #[case(AvailableImageStatus::Received, AstarteData::String("Received".to_string()))]
    #[case(AvailableImageStatus::Pulling, AstarteData::String("Pulling".to_string()))]
    #[case(AvailableImageStatus::Pulled, AstarteData::String("Pulled".to_string()))]
    #[case(AvailableImageStatus::Deleting, AstarteData::String("Deleting".to_string()))]
    #[case(AvailableImageStatus::Error, AstarteData::String("Error".to_string()))]
    fn should_convert_status_to_astarte_data_string(
        #[case] image_status: AvailableImageStatus,
        #[case] astarte_data_status: AstarteData,
    ) {
        assert_eq!(astarte_data_status, AstarteData::from(image_status));
    }

    #[rstest]
    #[case(AvailableImageStatus::Missing, false)]
    #[case(AvailableImageStatus::Received, false)]
    #[case(AvailableImageStatus::Pulling, false)]
    #[case(AvailableImageStatus::Pulled, true)]
    #[case(AvailableImageStatus::Deleting, true)]
    #[case(AvailableImageStatus::Error, false)]
    fn should_check_if_pulled(#[case] image_status: AvailableImageStatus, #[case] is_pulled: bool) {
        assert_eq!(is_pulled, image_status.is_pulled());
    }

    #[tokio::test]
    async fn should_store_image() {
        let id = Uuid::new_v4();

        let statuses = [
            AvailableImageStatus::Missing,
            AvailableImageStatus::Received,
            AvailableImageStatus::Pulling,
            AvailableImageStatus::Pulled,
            AvailableImageStatus::Deleting,
            AvailableImageStatus::Error,
        ];
        for status in statuses {
            let image = AvailableImage::new(&id);

            let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
            let mut seq = Sequence::new();

            expect_available_image_send(&mut client, &mut seq, image.clone(), status);

            image.send(&mut client, status).await.unwrap();
        }
    }

    #[tokio::test]
    async fn should_send_status() {
        let id = Uuid::new_v4();

        let statuses = [
            AvailableImageStatus::Missing,
            AvailableImageStatus::Received,
            AvailableImageStatus::Pulling,
            AvailableImageStatus::Pulled,
            AvailableImageStatus::Deleting,
            AvailableImageStatus::Error,
        ];
        for status in statuses {
            let image = AvailableImage::new(&id);

            let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
            let mut seq = Sequence::new();

            expect_status_send(&mut client, &mut seq, image.clone(), status);

            image.send_status(&mut client, status).await.unwrap();
        }
    }

    #[tokio::test]
    async fn should_store_image_with_fields() {
        let id = Uuid::new_v4();

        let image = AvailableImage {
            id: &id,
            image_id: Some("sha256:123456".to_string()),
            repo_tags: Some(Vec::new()),
            registry_digest: Some(Vec::new()),
            size: Some(1),
        };

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        expect_available_image_send(
            &mut client,
            &mut seq,
            image.clone(),
            AvailableImageStatus::Pulled,
        );

        image
            .send(&mut client, AvailableImageStatus::Pulled)
            .await
            .unwrap();
    }

    #[test]
    fn should_create_with_inspect() {
        let id = Uuid::new_v4();

        let inspect = ImageInspect {
            id: Some("sha256:123456".to_string()),
            repo_tags: Some(Vec::new()),
            repo_digests: Some(Vec::new()),
            size: Some(1),

            ..Default::default()
        };

        let image = AvailableImage::with_inspect(&id, inspect);

        assert_eq!(
            image,
            AvailableImage {
                id: &id,
                image_id: Some("sha256:123456".to_string()),
                repo_tags: Some(Vec::new()),
                registry_digest: Some(Vec::new()),
                size: Some(1),
            }
        );
    }

    #[tokio::test]
    async fn should_unset_image() {
        let id = Uuid::new_v4();

        let image = AvailableImage::new(&id);

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        expect_available_image_unset(&mut client, &mut seq, &id);

        image.unset(&mut client).await.unwrap();
    }
}
