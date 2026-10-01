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

use diesel::query_dsl::methods::{FilterDsl, SelectDsl};
use diesel::{
    ExpressionMethods, OptionalExtension, RunQueryDsl, delete, insert_or_ignore_into, update,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::file_bind::EnvFile;
use edgehog_store::models::containers::file_bind::{ContainerMissingEnvFile, EnvFileStatus};
use edgehog_store::schema::containers::{container_env_files, env_files};
use tracing::{error, instrument};
use uuid::Uuid;

use crate::requests::env_file::CreateEnvFile;

use super::{Result, StateStore, StoreError};

impl StateStore {
    /// Stores the env file received from the CreateRequest
    #[instrument(skip_all, fields(%create_env_file.id))]
    pub(crate) async fn create_env_file(&self, create_env_file: CreateEnvFile) -> Result<()> {
        let value = EnvFile::try_from(create_env_file)?;

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(env_files::table)
                    .values(&value)
                    .execute(writer)?;

                insert_or_ignore_into(container_env_files::table)
                    .values(ContainerMissingEnvFile::find_by_env_file(&value.id))
                    .execute(writer)?;

                delete(ContainerMissingEnvFile::find_by_env_file(&value.id)).execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip_all, fields(%id))]
    pub(crate) async fn find_env_file(&self, id: Uuid) -> Result<Option<EnvFile<'static>>> {
        let env_file = self
            .handle
            .for_read(move |reader| {
                EnvFile::find_id(&SqlUuid::new(id))
                    .first::<EnvFile>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await?;

        Ok(env_file)
    }

    /// Updates the state of a env_file
    #[instrument(skip(self))]
    pub(crate) async fn update_env_file_status(
        &self,
        env_file_id: Uuid,
        status: EnvFileStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(EnvFile::find_id(&SqlUuid::new(env_file_id)))
                    .set(env_files::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a env file
    #[instrument(skip(self))]
    pub(crate) async fn delete_env_file(&self, env_file_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated =
                    delete(EnvFile::find_id(&SqlUuid::new(env_file_id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_env_files_to_publish(&self) -> Result<Vec<SqlUuid>> {
        let env_files = self
            .handle
            .for_read(move |reader| {
                let env_files = env_files::table
                    .select(env_files::id)
                    .filter(env_files::status.eq(EnvFileStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(env_files)
            })
            .await?;

        Ok(env_files)
    }
}

impl TryFrom<CreateEnvFile> for EnvFile<'static> {
    type Error = StoreError;

    fn try_from(value: CreateEnvFile) -> Result<Self> {
        let CreateEnvFile {
            id,
            deployment_id: _,
            target_id,
            target_type,
        } = value;

        let target_type = target_type.parse().map_err(|()| {
            error!(target_type, "couldn't parse TargetType");

            StoreError::Conversion {
                ctx: "couldn't parse target type".to_string(),
            }
        })?;

        Ok(EnvFile {
            id: SqlUuid(id.0),
            status: EnvFileStatus::default(),
            target_id: target_id.into(),
            target_type,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::requests::env_file::tests::create_env_file_req;

    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    pub(crate) fn env_file_to_store(env_file: CreateEnvFile) -> EnvFile<'static> {
        EnvFile {
            id: env_file.id.0.into(),
            status: EnvFileStatus::Received,
            target_id: env_file.target_id.into(),
            target_type: env_file.target_type.parse().unwrap(),
        }
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();

        let res = store.find_env_file(env_file.id.0).await.unwrap().unwrap();

        let exp = env_file_to_store(env_file);

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();

        store
            .update_env_file_status(env_file.id.0, EnvFileStatus::Published)
            .await
            .unwrap();

        let res = store.find_env_file(env_file.id.0).await.unwrap().unwrap();

        let mut exp = env_file_to_store(env_file);
        exp.status = EnvFileStatus::Published;

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_load_for_publish() {
        let tmp = TempDir::with_prefix("update_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();

        let res = store.load_env_files_to_publish().await.unwrap();

        let exp = [SqlUuid::new(env_file.id.0)];

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_not_load_puslished() {
        let tmp = TempDir::with_prefix("update_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();
        store
            .update_env_file_status(env_file.id.0, EnvFileStatus::Published)
            .await
            .unwrap();

        let res = store.load_env_files_to_publish().await.unwrap();

        assert!(res.is_empty());
    }

    #[tokio::test]
    async fn should_delete() {
        let tmp = TempDir::with_prefix("update_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();

        store.delete_env_file(env_file.id.0).await.unwrap();

        let res = store.load_env_files_to_publish().await.unwrap();

        assert!(res.is_empty());
    }
}
