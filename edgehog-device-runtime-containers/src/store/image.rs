// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use diesel::{delete, insert_or_ignore_into, ExpressionMethods, OptionalExtension, RunQueryDsl};
use diesel::{update, QueryDsl};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::image::ImageStatus;
use edgehog_store::models::QueryModel;
use edgehog_store::{
    models::containers::{container::ContainerMissingImage, image::Image},
    schema::containers::{container_missing_images, containers, images},
};
use tracing::instrument;
use uuid::Uuid;

use crate::docker::image::Image as ContainerImage;
use crate::requests::image::CreateImage;
use crate::resource::image::ImageResource;

use super::{Result, StateStore};

impl StateStore {
    /// Stores the image received from the CreateRequest
    #[instrument(skip_all, fields(%image.id))]
    pub(crate) async fn create_image(&self, image: CreateImage) -> Result<()> {
        let image = Image::from(image);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(images::table)
                    .values(&image)
                    .execute(writer)?;

                update(containers::table)
                    .set(containers::image_id.eq(image.id))
                    .filter(
                        containers::id.eq_any(
                            ContainerMissingImage::find_by_image(&image.id)
                                .select(container_missing_images::container_id),
                        ),
                    )
                    .execute(writer)?;

                delete(ContainerMissingImage::find_by_image(&image.id)).execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the local image id
    #[instrument(skip(self))]
    pub(crate) async fn set_image_id(&self, id: Uuid, local_id: String) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Image::find_id(&SqlUuid::new(id)))
                    .set(images::local_id.eq(local_id))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of a image
    #[instrument(skip(self))]
    pub(crate) async fn update_image_status(&self, id: Uuid, status: ImageStatus) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Image::find_id(&SqlUuid::new(id)))
                    .set(images::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of a image
    #[instrument(skip(self))]
    pub(crate) async fn update_image_local_id(
        &self,
        id: Uuid,
        local_id: Option<String>,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Image::find_id(&SqlUuid::new(id)))
                    .set(images::local_id.eq(local_id))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Delete the [`Image`] with the give [id](Uuid).
    #[instrument(skip(self))]
    pub(crate) async fn delete_image(&self, id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(Image::find_id(&SqlUuid::new(id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Fetches an image by id
    #[instrument(skip(self))]
    pub(crate) async fn image(&mut self, id: Uuid) -> Result<Option<ImageResource>> {
        let image = self
            .handle
            .for_read(move |reader| {
                let image: Option<Image> =
                    Image::find_id(&SqlUuid::new(id)).first(reader).optional()?;

                Ok(image)
            })
            .await?
            .map(|img| ImageResource::new(ContainerImage::from(img)));

        Ok(image)
    }

    /// Fetches an image by id
    #[instrument(skip(self))]
    pub(crate) async fn all_images(&mut self) -> Result<Vec<Image>> {
        let image = self
            .handle
            .for_read(move |reader| {
                let images = images::table.load(reader)?;

                Ok(images)
            })
            .await?;

        Ok(image)
    }

    /// Fetches an image by id
    #[instrument(skip(self))]
    pub(crate) async fn unpublished_images(&mut self) -> Result<Vec<SqlUuid>> {
        let image = self
            .handle
            .for_read(move |reader| {
                let images = images::table
                    .select(images::id)
                    .filter(images::status.eq(ImageStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(images)
            })
            .await?;

        Ok(image)
    }

    /// Fetches the images that need to be published
    #[instrument(skip(self))]
    pub(crate) async fn to_publish_images(&mut self) -> Result<Vec<Image>> {
        let image = self
            .handle
            .for_read(move |reader| {
                let images = images::table.load(reader)?;

                Ok(images)
            })
            .await?;

        Ok(image)
    }
}

impl From<CreateImage> for Image {
    fn from(
        CreateImage {
            id,
            reference,
            registry_auth,
        }: CreateImage,
    ) -> Self {
        let registry_auth = (!registry_auth.is_empty()).then_some(registry_auth);

        Self {
            id: SqlUuid::new(id),
            local_id: None,
            status: ImageStatus::default(),
            reference,
            registry_auth,
        }
    }
}

impl From<Image> for ContainerImage {
    fn from(value: Image) -> Self {
        Self {
            id: value.local_id,
            reference: value.reference,
            registry_auth: value.registry_auth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::image::tests::mock_image_req;

    async fn find_image(store: &mut StateStore, id: Uuid) -> Option<Image> {
        store
            .handle
            .for_read(move |reader| {
                Image::find_id(&SqlUuid::new(id))
                    .first::<Image>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_image").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let image_id = Uuid::new_v4();
        let image = mock_image_req(image_id, "postgres:15", "");
        store.create_image(image).await.unwrap();

        let res = find_image(&mut store, image_id).await.unwrap();

        let exp = Image {
            id: SqlUuid::new(image_id),
            local_id: None,
            status: ImageStatus::Received,
            reference: "postgres:15".to_string(),
            registry_auth: None,
        };

        assert_eq!(res, exp)
    }

    #[tokio::test]
    async fn all_images() {
        let tmp = TempDir::with_prefix("store_image").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let image1_id = Uuid::new_v4();
        let image = mock_image_req(image1_id, "postgres:15", "");
        store.create_image(image).await.unwrap();

        let image2_id = Uuid::new_v4();
        let image = mock_image_req(image2_id, "alpine:3.21", "");
        store.create_image(image).await.unwrap();

        let res = store.all_images().await.unwrap();

        let exp = vec![
            Image {
                id: SqlUuid::new(image1_id),
                local_id: None,
                status: ImageStatus::Received,
                reference: "postgres:15".to_string(),
                registry_auth: None,
            },
            Image {
                id: SqlUuid::new(image2_id),
                local_id: None,
                status: ImageStatus::Received,
                reference: "alpine:3.21".to_string(),
                registry_auth: None,
            },
        ];

        assert_eq!(res, exp)
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_image").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let image_id = Uuid::new_v4();
        let image = mock_image_req(image_id, "postgres:15", "");
        store.create_image(image).await.unwrap();
        store
            .update_image_status(image_id, ImageStatus::Published)
            .await
            .unwrap();
        let local_id = Uuid::new_v4().to_string();
        store
            .set_image_id(image_id, local_id.clone())
            .await
            .unwrap();

        let res = find_image(&mut store, image_id).await.unwrap();

        let exp = Image {
            id: SqlUuid::new(image_id),
            local_id: Some(local_id),
            status: ImageStatus::Published,
            reference: "postgres:15".to_string(),
            registry_auth: None,
        };

        assert_eq!(res, exp)
    }
}
