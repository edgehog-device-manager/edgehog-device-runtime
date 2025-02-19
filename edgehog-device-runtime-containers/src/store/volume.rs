// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::collections::HashMap;

use diesel::{delete, insert_or_ignore_into, ExpressionMethods, OptionalExtension, RunQueryDsl};
use diesel::{update, QueryDsl};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::container::ContainerMissingVolume;
use edgehog_store::models::containers::volume::{Volume, VolumeDriverOpts, VolumeStatus};
use edgehog_store::models::QueryModel;
use edgehog_store::schema::containers::{container_volumes, volume_driver_opts, volumes};
use itertools::Itertools;
use tracing::instrument;
use uuid::Uuid;

use crate::docker::volume::Volume as ContainerVolume;
use crate::requests::volume::CreateVolume;
use crate::resource::volume::VolumeResource;

use super::{split_key_value, Result, StateStore, StoreError};

impl StateStore {
    /// Stores the volume received from the CreateRequest
    #[instrument(skip_all, fields(%volume.id))]
    pub(crate) async fn create_volume(&self, volume: CreateVolume) -> Result<()> {
        let opts = Vec::<VolumeDriverOpts>::try_from(&volume)?;
        let volume = Volume::from(volume);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(volumes::table)
                    .values(&volume)
                    .execute(writer)?;

                insert_or_ignore_into(volume_driver_opts::table)
                    .values(opts)
                    .execute(writer)?;

                insert_or_ignore_into(container_volumes::table)
                    .values(ContainerMissingVolume::find_by_volume(&volume.id))
                    .execute(writer)?;

                delete(ContainerMissingVolume::find_by_volume(&volume.id)).execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of a volume
    #[instrument(skip(self))]
    pub(crate) async fn update_volume_status(&self, id: Uuid, status: VolumeStatus) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Volume::find_id(&SqlUuid::new(id)))
                    .set(volumes::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Delete the [`Volume`] with the given [`id`](Uuid).
    #[instrument(skip(self))]
    pub(crate) async fn delete_volume(&self, id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(Volume::find_id(&SqlUuid::new(id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_volumes_to_publish(&mut self) -> Result<Vec<SqlUuid>> {
        let volumes = self
            .handle
            .for_read(move |reader| {
                let volumes = volumes::table
                    .select(volumes::id)
                    .filter(volumes::status.eq(VolumeStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(volumes)
            })
            .await?;

        Ok(volumes)
    }

    /// Fetches an volume by id
    #[instrument(skip(self))]
    pub(crate) async fn find_volume(&mut self, id: Uuid) -> Result<Option<VolumeResource>> {
        let volume = self
            .handle
            .for_read(move |reader| {
                let Some(volume): Option<Volume> = Volume::find_id(&SqlUuid::new(id))
                    .first(reader)
                    .optional()?
                else {
                    return Ok(None);
                };

                let driver_opts: HashMap<String, String> = volume_driver_opts::table
                    .filter(volume_driver_opts::volume_id.eq(&SqlUuid::new(id)))
                    .load::<VolumeDriverOpts>(reader)?
                    .into_iter()
                    .map(|opt| (opt.name, opt.value))
                    .collect();

                Ok(Some(VolumeResource::new(ContainerVolume::new(
                    volume.id.to_string(),
                    volume.driver,
                    driver_opts,
                ))))
            })
            .await?;

        Ok(volume)
    }
}

impl From<CreateVolume> for Volume {
    fn from(
        CreateVolume {
            id,
            deployment_id: _,
            driver,
            options: _,
        }: CreateVolume,
    ) -> Self {
        Self {
            id: SqlUuid::new(id),
            status: VolumeStatus::default(),
            driver,
        }
    }
}

impl TryFrom<&CreateVolume> for Vec<VolumeDriverOpts> {
    type Error = StoreError;

    fn try_from(value: &CreateVolume) -> std::result::Result<Self, Self::Error> {
        let volume_id = SqlUuid::new(value.id);

        value
            .options
            .iter()
            .map(|s| {
                split_key_value(s)
                    .map(|(name, value)| VolumeDriverOpts {
                        volume_id,
                        name: name.to_string(),
                        // NOTE: Default to empty string, this is a sane approach even if the
                        //       behaviour is not directly specified in the Docker docs.
                        value: value.unwrap_or_default().to_string(),
                    })
                    .ok_or(StoreError::ParseKeyValue {
                        ctx: "volume driver options",
                        value: s.to_string(),
                    })
            })
            .try_collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::requests::ReqUuid;

    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    async fn find_volume(store: &mut StateStore, id: Uuid) -> Option<Volume> {
        store
            .handle
            .for_read(move |reader| {
                Volume::find_id(&SqlUuid::new(id))
                    .first(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    impl StateStore {
        pub(crate) async fn volume_opts(
            &mut self,
            volume_id: Uuid,
        ) -> Result<Vec<VolumeDriverOpts>> {
            let volume = self
                .handle
                .for_read(move |reader| {
                    let volume: Vec<VolumeDriverOpts> = volume_driver_opts::table
                        .filter(volume_driver_opts::volume_id.eq(SqlUuid::new(volume_id)))
                        .load(reader)?;

                    Ok(volume)
                })
                .await?;

            Ok(volume)
        }
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_volume").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let volume_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let volume = CreateVolume {
            id: ReqUuid(volume_id),
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let res = find_volume(&mut store, volume_id).await.unwrap();

        let exp = Volume {
            id: SqlUuid::new(volume_id),
            status: VolumeStatus::Received,
            driver: "local".to_string(),
        };

        assert_eq!(res, exp);

        let volume_opts = store.volume_opts(volume_id).await.unwrap();

        assert_eq!(
            volume_opts,
            vec![
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "device".to_string(),
                    value: "tmpfs".to_string()
                },
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "o".to_string(),
                    value: "size=100m,uid=1000".to_string()
                },
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "type".to_string(),
                    value: "tmpfs".to_string()
                }
            ]
        );
    }

    #[tokio::test]
    async fn should_store_empty_option() {
        let tmp = TempDir::with_prefix("store_volume").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let volume_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let volume = CreateVolume {
            id: ReqUuid(volume_id),
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: [
                "device=tmpfs",
                "o=size=100m,uid=1000",
                "type=tmpfs",
                "empty=",
            ]
            .map(str::to_string)
            .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let res = find_volume(&mut store, volume_id).await.unwrap();

        let exp = Volume {
            id: SqlUuid::new(volume_id),
            status: VolumeStatus::Received,
            driver: "local".to_string(),
        };

        assert_eq!(res, exp);

        let volume_opts = store.volume_opts(volume_id).await.unwrap();

        assert_eq!(
            volume_opts,
            vec![
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "device".to_string(),
                    value: "tmpfs".to_string()
                },
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "empty".to_string(),
                    value: String::new()
                },
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "o".to_string(),
                    value: "size=100m,uid=1000".to_string()
                },
                VolumeDriverOpts {
                    volume_id: SqlUuid::new(volume_id),
                    name: "type".to_string(),
                    value: "tmpfs".to_string()
                },
            ]
        );
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_volume").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let volume_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let volume = CreateVolume {
            id: ReqUuid(volume_id),
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        store
            .update_volume_status(volume_id, VolumeStatus::Published)
            .await
            .unwrap();

        let res = find_volume(&mut store, volume_id).await.unwrap();

        let exp = Volume {
            id: SqlUuid::new(volume_id),
            status: VolumeStatus::Published,
            driver: "local".to_string(),
        };

        assert_eq!(res, exp);
    }
}
