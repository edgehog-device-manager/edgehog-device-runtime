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

use std::io;

use astarte_device_sdk::properties::PropAccess;
use edgehog_store::models::containers::file_bind::{EnvFile, EnvFileStatus, TargetType};
use tracing::{error, instrument};

use crate::properties::{AvailableProp, Client, env_file::AvailableEnvFile};
use crate::resource::ResourceError;

use self::parser::EnvReader;

use super::utils::file_store_device_path;
use super::{Context, Resource, Result};

pub(crate) mod parser;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnvFileResource<'a> {
    pub(crate) inner: EnvFile<'a>,
}

impl<'a> EnvFileResource<'a> {
    pub(crate) fn new(inner: EnvFile<'a>) -> Self {
        Self { inner }
    }

    #[instrument(skip_all, fields(id = %ctx.id))]
    pub(crate) async fn check<D>(ctx: &mut Context<'_, D>) -> Result<()>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        let Some(inner) = ctx.store.find_env_file(ctx.id).await? else {
            error!("env file not found in database");

            return Err(ResourceError::Missing {
                id: ctx.id,
                resource: "env file",
            });
        };

        let this = EnvFileResource { inner };

        let mut file = this.open(ctx).await?;

        file.validate()
            .await
            .map_err(|err| err.into_resource_err(this.inner.id.0))?;

        Ok(())
    }

    pub(crate) async fn path_on_device<D>(&self, ctx: &Context<'_, D>) -> Result<String>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        match self.inner.target_type {
            TargetType::Storage => {
                file_store_device_path(
                    ctx.device,
                    "env file",
                    self.inner.id.0,
                    &self.inner.target_id,
                )
                .await
            }
        }
    }

    pub(crate) async fn open<D>(&self, ctx: &Context<'_, D>) -> Result<EnvReader>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        let path_on_device = self.path_on_device(ctx).await?;

        match tokio::fs::File::open(&path_on_device).await {
            Ok(file) => Ok(EnvReader::new(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                error!("env file doesn't exits");

                Err(ResourceError::Missing {
                    id: self.inner.id.0,
                    resource: "env file",
                })
            }
            Err(error) => {
                error!(%error, "couldn't check if env file exists");

                Err(ResourceError::Invalid {
                    id: self.inner.id.0,
                    resource: "env file",
                    ctx: "couldn't check if env file exists",
                })
            }
        }
    }
}

impl<'a, D> Resource<D> for EnvFileResource<'a>
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableEnvFile::new(&ctx.id)
            .send(ctx.device, true)
            .await?;

        ctx.store
            .update_env_file_status(ctx.id, EnvFileStatus::Published)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};
    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::Client;
    use crate::requests::env_file::tests::create_env_file_req;
    use crate::store::StateStore;

    use super::*;

    #[tokio::test]
    async fn should_get_path_on_device() {
        let tmp = TempDir::with_prefix("update_env_file").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let path_on_device = tmp.path().join("path_on_device");

        tokio::fs::write(&path_on_device, &[]).await.unwrap();

        let handle = edgehog_store::db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let env_file = create_env_file_req(deployment_id);
        store.create_env_file(env_file.clone()).await.unwrap();

        let client = Client::default();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        device
            .expect_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.storage.File"),
                predicate::eq(format!("/{}/pathOnDevice", env_file.target_id)),
            )
            .returning({
                let path_on_device = path_on_device.to_string_lossy().to_string();

                move |_, _| Ok(Some(AstarteData::String(path_on_device.clone())))
            });

        let mut ctx = Context {
            id: env_file.id.0,
            store: &mut store,
            device: &mut device,
            client: &client,
        };

        EnvFileResource::check(&mut ctx).await.unwrap();
    }
}
