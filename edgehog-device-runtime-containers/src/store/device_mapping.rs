// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use diesel::query_dsl::methods::{FilterDsl, SelectDsl};
use diesel::{delete, insert_or_ignore_into, update, ExpressionMethods, RunQueryDsl};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::container::ContainerMissingDeviceMapping;
use edgehog_store::models::containers::device_mapping::DeviceMapping;
use edgehog_store::models::containers::device_mapping::DeviceMappingStatus;
use edgehog_store::models::QueryModel;
use edgehog_store::schema::containers::{container_device_mappings, device_mappings};
use tracing::instrument;
use uuid::Uuid;

use crate::requests::device_mapping::CreateDeviceMapping;

use super::{Result, StateStore};

impl StateStore {
    /// Stores the device mapping received from the CreateRequest
    #[instrument(skip_all, fields(%create_device_mapping.id))]
    pub(crate) async fn create_device_mapping(
        &self,
        create_device_mapping: CreateDeviceMapping,
    ) -> Result<()> {
        let dm_value = DeviceMapping::from(create_device_mapping);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(device_mappings::table)
                    .values(&dm_value)
                    .execute(writer)?;

                insert_or_ignore_into(container_device_mappings::table)
                    .values(ContainerMissingDeviceMapping::find_by_device_mapping(
                        &dm_value.id,
                    ))
                    .execute(writer)?;

                delete(ContainerMissingDeviceMapping::find_by_device_mapping(
                    &dm_value.id,
                ))
                .execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the state of a device_mapping
    #[instrument(skip(self))]
    pub(crate) async fn update_device_mapping_status(
        &self,
        device_mapping_id: Uuid,
        status: DeviceMappingStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(DeviceMapping::find_id(&SqlUuid::new(device_mapping_id)))
                    .set(device_mappings::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a [`DeviceMapping`]
    #[instrument(skip(self))]
    pub(crate) async fn delete_device_mapping(&self, device_mapping_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(DeviceMapping::find_id(&SqlUuid::new(device_mapping_id)))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_device_mappings_to_publish(&self) -> Result<Vec<SqlUuid>> {
        let device_mappings = self
            .handle
            .for_read(move |reader| {
                let device_mappings = device_mappings::table
                    .select(device_mappings::id)
                    .filter(device_mappings::status.eq(DeviceMappingStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(device_mappings)
            })
            .await?;

        Ok(device_mappings)
    }
}

impl From<CreateDeviceMapping> for DeviceMapping {
    fn from(
        CreateDeviceMapping {
            id,
            deployment_id: _,
            path_on_host,
            path_in_container,
            c_group_permissions,
        }: CreateDeviceMapping,
    ) -> Self {
        Self {
            id: SqlUuid::new(id),
            status: DeviceMappingStatus::default(),
            path_on_host,
            path_in_container,
            cgroup_permissions: c_group_permissions.into(),
        }
    }
}

impl From<DeviceMapping> for crate::docker::container::DeviceMapping {
    fn from(
        DeviceMapping {
            id: _,
            status: _,
            path_on_host,
            path_in_container,
            cgroup_permissions,
        }: DeviceMapping,
    ) -> Self {
        Self {
            path_on_host,
            path_in_container,
            cgroup_permissions,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::requests::{OptString, ReqUuid};

    use super::*;

    use diesel::OptionalExtension;
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    async fn find_device_mapping(store: &StateStore, id: Uuid) -> Option<DeviceMapping> {
        store
            .handle
            .for_read(move |reader| {
                DeviceMapping::find_id(&SqlUuid::new(id))
                    .first::<DeviceMapping>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_device_mapping").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let device_mapping_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let device_mapping = CreateDeviceMapping {
            id: ReqUuid(device_mapping_id),
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "/dev/tty12".to_string(),
            c_group_permissions: OptString::from("mvr".to_string()),
        };
        store.create_device_mapping(device_mapping).await.unwrap();

        let res = find_device_mapping(&store, device_mapping_id)
            .await
            .unwrap();

        let exp = DeviceMapping {
            id: SqlUuid::new(device_mapping_id),
            status: DeviceMappingStatus::Received,
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "/dev/tty12".to_string(),
            cgroup_permissions: Some("mvr".to_string()),
        };

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_device_mapping").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let device_mapping_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let device_mapping = CreateDeviceMapping {
            id: ReqUuid(device_mapping_id),
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "/dev/tty12".to_string(),
            c_group_permissions: OptString::from("mvr".to_string()),
        };
        store.create_device_mapping(device_mapping).await.unwrap();

        store
            .update_device_mapping_status(device_mapping_id, DeviceMappingStatus::Published)
            .await
            .unwrap();

        let res = find_device_mapping(&store, device_mapping_id)
            .await
            .unwrap();

        let exp = DeviceMapping {
            id: SqlUuid::new(device_mapping_id),
            status: DeviceMappingStatus::Published,
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "/dev/tty12".to_string(),
            cgroup_permissions: Some("mvr".to_string()),
        };

        assert_eq!(res, exp);
    }
}
