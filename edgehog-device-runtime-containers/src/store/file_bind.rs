// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

use std::borrow::Cow;

use diesel::query_dsl::methods::{FilterDsl, SelectDsl};
use diesel::{
    ExpressionMethods, OptionalExtension, RunQueryDsl, delete, insert_or_ignore_into, update,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::file_bind::FileBind;
use edgehog_store::models::containers::file_bind::{ContainerMissingFileBind, FileBindStatus};
use edgehog_store::schema::containers::{container_file_binds, file_binds};
use tracing::{error, instrument};
use uuid::Uuid;

use crate::requests::file_bind::CreateFileBind;

use super::{Result, StateStore, StoreError};

impl StateStore {
    /// Stores the file bind received from the CreateRequest
    #[instrument(skip_all, fields(%create_file_bind.id))]
    pub(crate) async fn create_file_bind(&self, create_file_bind: CreateFileBind) -> Result<()> {
        let value = FileBind::try_from(create_file_bind)?;

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(file_binds::table)
                    .values(&value)
                    .execute(writer)?;

                insert_or_ignore_into(container_file_binds::table)
                    .values(ContainerMissingFileBind::find_by_file_bind(&value.id))
                    .execute(writer)?;

                delete(ContainerMissingFileBind::find_by_file_bind(&value.id)).execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip_all, fields(%id))]
    pub(crate) async fn find_file_bind(&self, id: Uuid) -> Result<Option<FileBind<'static>>> {
        let file_bind = self
            .handle
            .for_read(move |reader| {
                FileBind::find_id(&SqlUuid::new(id))
                    .first::<FileBind>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await?;

        Ok(file_bind)
    }

    /// Updates the state of a file_bind
    #[instrument(skip(self))]
    pub(crate) async fn update_file_bind_status(
        &self,
        file_bind_id: Uuid,
        status: FileBindStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(FileBind::find_id(&SqlUuid::new(file_bind_id)))
                    .set(file_binds::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a file bind
    #[instrument(skip(self))]
    pub(crate) async fn delete_file_bind(&self, file_bind_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated =
                    delete(FileBind::find_id(&SqlUuid::new(file_bind_id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_file_binds_to_publish(&self) -> Result<Vec<SqlUuid>> {
        let file_binds = self
            .handle
            .for_read(move |reader| {
                let file_binds = file_binds::table
                    .select(file_binds::id)
                    .filter(file_binds::status.eq(FileBindStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(file_binds)
            })
            .await?;

        Ok(file_binds)
    }
}

impl TryFrom<CreateFileBind> for FileBind<'static> {
    type Error = StoreError;

    fn try_from(value: CreateFileBind) -> Result<Self> {
        let CreateFileBind {
            id,
            deployment_id: _,
            target_id,
            target_type,
            mountpoint,
            options,
        } = value;

        let target_type = target_type.parse().map_err(|()| {
            error!(target_type, "couldn't parse TargetType");

            StoreError::Conversion {
                ctx: "couldn't parse target type".to_string(),
            }
        })?;

        let options = options.and_then(Option::<Cow<str>>::from);

        Ok(FileBind {
            id: SqlUuid(id.0),
            status: FileBindStatus::default(),
            target_id: target_id.into(),
            target_type,
            mountpoint: mountpoint.into(),
            options,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::requests::file_bind::tests::create_file_bind_req;

    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    pub(crate) fn file_bind_to_store(file_bind: CreateFileBind) -> FileBind<'static> {
        FileBind {
            id: file_bind.id.0.into(),
            status: FileBindStatus::Received,
            target_id: file_bind.target_id.into(),
            target_type: file_bind.target_type.parse().unwrap(),
            mountpoint: file_bind.mountpoint.into(),
            options: file_bind.options.and_then(Option::<Cow<str>>::from),
        }
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();

        let res = store.find_file_bind(file_bind.id.0).await.unwrap().unwrap();

        let exp = file_bind_to_store(file_bind);

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();

        store
            .update_file_bind_status(file_bind.id.0, FileBindStatus::Published)
            .await
            .unwrap();

        let res = store.find_file_bind(file_bind.id.0).await.unwrap().unwrap();

        let mut exp = file_bind_to_store(file_bind);
        exp.status = FileBindStatus::Published;

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_load_for_publish() {
        let tmp = TempDir::with_prefix("update_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();

        let res = store.load_file_binds_to_publish().await.unwrap();

        let exp = [SqlUuid::new(file_bind.id.0)];

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_not_load_puslished() {
        let tmp = TempDir::with_prefix("update_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();
        store
            .update_file_bind_status(file_bind.id.0, FileBindStatus::Published)
            .await
            .unwrap();

        let res = store.load_file_binds_to_publish().await.unwrap();

        assert!(res.is_empty());
    }

    #[tokio::test]
    async fn should_delete() {
        let tmp = TempDir::with_prefix("update_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();

        store.delete_file_bind(file_bind.id.0).await.unwrap();

        let res = store.load_file_binds_to_publish().await.unwrap();

        assert!(res.is_empty());
    }
}
