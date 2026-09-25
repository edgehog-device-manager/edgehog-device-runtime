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

use astarte_device_sdk::properties::PropAccess;
use edgehog_store::models::containers::file_bind::{FileBind, FileBindStatus, TargetType};
use tracing::{error, instrument};

use crate::properties::{AvailableProp, Client, file_bind::AvailableFileBind};

use super::utils::file_store_device_path;
use super::{Context, Resource, ResourceError, Result};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileBindResource<'a> {
    inner: FileBind<'a>,
}

impl<'a> FileBindResource<'a> {
    pub(crate) fn new(inner: FileBind<'a>) -> Self {
        Self { inner }
    }

    #[instrument(skip_all, fields(id = %ctx.id))]
    pub(crate) async fn check<D>(ctx: &mut Context<'_, D>) -> Result<()>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        let Some(inner) = ctx.store.find_file_bind(ctx.id).await? else {
            error!("file bind not found in database");

            return Err(ResourceError::Missing {
                id: ctx.id,
                resource: "file bind",
            });
        };

        let this = FileBindResource { inner };

        let path_on_device = this.path_on_device(ctx).await?;

        match tokio::fs::try_exists(path_on_device).await {
            Ok(true) => {}
            Ok(false) => {
                error!("file bind doesn't exits");

                return Err(ResourceError::Missing {
                    id: ctx.id,
                    resource: "file bind",
                });
            }
            Err(error) => {
                error!(%error, "couldn't check if file bind exists");

                return Err(ResourceError::Invalid {
                    id: ctx.id,
                    resource: "file bind",
                    ctx: "couldn't check if file bind exists",
                });
            }
        }

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
                    "file bind",
                    self.inner.id.0,
                    &self.inner.target_id,
                )
                .await
            }
        }
    }

    pub(crate) async fn to_container_bind<D>(&self, ctx: &Context<'_, D>) -> Result<String>
    where
        D: PropAccess + Send + Sync + 'static,
    {
        let path_on_device = self.path_on_device(ctx).await?;

        // The container runtime requires absolute paths.
        let path_on_device = tokio::fs::canonicalize(path_on_device)
            .await
            .map_err(|error| {
                error!(%error, "couldn't canonicalize the file bind path");

                ResourceError::Invalid {
                    id: ctx.id,
                    resource: "file bind",
                    ctx: "couldn't canonicalize the file bind path",
                }
            })?;

        let path_on_device = path_on_device
            .to_str()
            .ok_or_else(|| ResourceError::Invalid {
                id: ctx.id,
                resource: "file bind",
                ctx: "path is not UTF-8",
            })?;

        let bind = match &self.inner.options {
            Some(opt) => {
                format!("{path_on_device}:{}:{opt}", self.inner.mountpoint)
            }
            None => {
                format!("{path_on_device}:{}", self.inner.mountpoint)
            }
        };

        Ok(bind)
    }
}

impl<'a, D> Resource<D> for FileBindResource<'a>
where
    D: Client + Send + Sync + 'static,
{
    #[instrument(skip_all, fields(id = %ctx.id))]
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableFileBind::new(&ctx.id)
            .send(ctx.device, true)
            .await?;

        ctx.store
            .update_file_bind_status(ctx.id, FileBindStatus::Published)
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
    use crate::requests::file_bind::tests::create_file_bind_req;
    use crate::store::StateStore;

    use super::*;

    #[tokio::test]
    async fn should_get_path_on_device() {
        let tmp = TempDir::with_prefix("update_file_bind").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let path_on_device = tmp.path().join("path_on_device");

        tokio::fs::write(&path_on_device, &[]).await.unwrap();

        let handle = edgehog_store::db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let file_bind = create_file_bind_req(deployment_id);
        store.create_file_bind(file_bind.clone()).await.unwrap();

        let client = Client::default();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        device
            .expect_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.storage.File"),
                predicate::eq(format!("/{}/pathOnDevice", file_bind.target_id)),
            )
            .returning({
                let path_on_device = path_on_device.to_string_lossy().to_string();

                move |_, _| Ok(Some(AstarteData::String(path_on_device.clone())))
            });

        let mut ctx = Context {
            id: file_bind.id.0,
            store: &mut store,
            device: &mut device,
            client: &client,
        };

        FileBindResource::check(&mut ctx).await.unwrap();
    }
}
