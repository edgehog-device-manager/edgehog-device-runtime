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

use diesel::dsl::exists;
use diesel::{
    CombineDsl, ExpressionMethods, NullableExpressionMethods, QueryDsl, RunQueryDsl,
    SqliteConnection, delete, insert_or_ignore_into, select, update,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::container::Container;
use edgehog_store::models::containers::deployment::{
    Deployment, DeploymentContainer, DeploymentMissingContainer, DeploymentStatus,
};
use edgehog_store::schema::containers::{
    container_device_mappings, container_device_requests, container_env_files,
    container_file_binds, container_missing_device_mappings, container_missing_device_requests,
    container_missing_env_files, container_missing_file_binds, container_missing_images,
    container_missing_networks, container_missing_volumes, container_networks, container_volumes,
    containers, deployment_containers, deployment_missing_containers, deployments,
};
use itertools::Itertools;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::requests::deployment::{CreateDeployment, DeploymentUpdate};
use crate::resource::deployment::Deployment as DeploymentResource;

use super::{Result, StateStore};

impl StateStore {
    /// Stores a received deployment
    #[instrument(skip_all, fields(%deployment.id))]
    pub(crate) async fn create_deployment(&self, deployment: CreateDeployment) -> Result<()> {
        let containers = deployment.containers.iter().map(SqlUuid::new).collect_vec();
        let deployment = Deployment::from(deployment);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(deployments::table)
                    .values(&deployment)
                    .execute(writer)?;

                for (idx, container_id) in containers.into_iter().enumerate() {
                    let idx = i64::try_from(idx).map_err(|error|{
                        tracing::error!(%error, %container_id, "couldn't convert index for deployment's container");

                        HandleError::Application(error.into())
                    })?;

                    let exists: bool = Container::exists(&container_id).get_result(writer)?;

                    if !exists {
                        insert_or_ignore_into(deployment_missing_containers::table)
                            .values(DeploymentMissingContainer {
                                deployment_id: deployment.id,
                                container_id,
                                idx,
                            })
                            .execute(writer)?;

                    } else {
                        insert_or_ignore_into(deployment_containers::table)
                            .values(DeploymentContainer {
                                deployment_id: deployment.id,
                                container_id,
                                idx,
                            })
                            .execute(writer)?;
                    }
                }

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of a deployment
    #[instrument(skip(self))]
    pub(crate) async fn update_deployment_status(
        &self,
        id: Uuid,
        status: DeploymentStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Deployment::find_id(&SqlUuid::new(id)))
                    .set(deployments::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn delete_deployment(&self, id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(Deployment::find_id(&SqlUuid::new(id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of two deployment atomically
    #[instrument(skip(self))]
    pub(crate) async fn deployment_update(&self, from: Uuid, to: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let status: DeploymentStatus = Deployment::find_id(&SqlUuid::new(from))
                    .select(deployments::status)
                    .first(writer)?;

                // Do nothing if the status is not started
                match status {
                    DeploymentStatus::Started => {
                        let updated = update(Deployment::find_id(&SqlUuid::new(from)))
                            .set(deployments::status.eq(DeploymentStatus::Stopped))
                            .execute(writer)?;

                        HandleError::check_modified(updated, 1)?;

                        debug!("deployment to update set to stopped")
                    }
                    DeploymentStatus::Received
                    | DeploymentStatus::Stopped
                    | DeploymentStatus::Deleted => {
                        debug!("deployment to update is in state {status}, not setting to stopped")
                    }
                }

                let updated = update(Deployment::find_id(&SqlUuid::new(to)))
                    .set(deployments::status.eq(DeploymentStatus::Started))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_deployments_in(
        &self,
        status: DeploymentStatus,
    ) -> Result<Vec<SqlUuid>> {
        let deployment = self
            .handle
            .for_read(move |reader| {
                let deployments = deployments::table
                    .filter(deployments::status.eq(status))
                    .select(deployments::id)
                    .load::<SqlUuid>(reader)?;

                Ok(deployments)
            })
            .await?;

        Ok(deployment)
    }

    /// Fetches the containers for a deployment
    #[instrument(skip(self))]
    pub(crate) async fn load_deployment_containers(
        &self,
        id: Uuid,
    ) -> Result<Option<Vec<SqlUuid>>> {
        let containers = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);
                if !Deployment::exists(&id).get_result(reader)? {
                    return Ok(None);
                }

                let containers = deployment_containers::table
                    .select(deployment_containers::container_id)
                    .filter(deployment_containers::deployment_id.eq(id))
                    .order_by(deployment_containers::idx)
                    .load::<SqlUuid>(reader)?;

                Ok(Some(containers))
            })
            .await?;

        Ok(containers)
    }

    pub(crate) async fn find_complete_deployment(
        &self,
        id: Uuid,
    ) -> Result<Option<DeploymentResource>> {
        let deployment = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);

                if !Deployment::exists(&id).get_result(reader)? {
                    return Ok(None);
                }

                if !is_deployment_complete(reader, &id)? {
                    return Ok(None);
                }

                let containers = deployment_containers::table
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select((
                        deployment_containers::idx,
                        deployment_containers::container_id,
                    ))
                    .order_by(deployment_containers::idx)
                    .load::<(i64, SqlUuid)>(reader)?
                    .into_iter()
                    .map(|(idx, id)| (idx, Uuid::from(id)))
                    .collect();

                let images = deployment_containers::table
                    .inner_join(containers::table)
                    .filter(deployment_containers::deployment_id.eq(id))
                    .filter(containers::image_id.is_not_null())
                    .select(containers::image_id.assume_not_null())
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let volumes = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_volumes::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_volumes::volume_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let networks = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_networks::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_networks::network_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let device_mappings = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_device_mappings::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_device_mappings::device_mapping_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let device_requests = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_device_requests::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_device_requests::device_request_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let file_binds = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_file_binds::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_file_binds::file_bind_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let env_files = deployment_containers::table
                    .inner_join(containers::table.inner_join(container_env_files::table))
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_env_files::env_file_id)
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                Ok(Some(DeploymentResource {
                    containers,
                    images,
                    volumes,
                    networks,
                    device_mappings,
                    device_requests,
                    file_binds,
                    env_files,
                }))
            })
            .await?;

        Ok(deployment)
    }

    pub(crate) async fn find_deployment_for_delete(
        &self,
        id: Uuid,
    ) -> Result<Option<DeploymentResource>> {
        let deployment = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);

                if !Deployment::exists(&id).get_result(reader)? {
                    return Ok(None);
                }

                // Delete only the resources present in this deployment
                let containers = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select((
                        deployment_containers::idx,
                        deployment_containers::container_id,
                    ))
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select((
                                deployment_containers::idx,
                                deployment_containers::container_id,
                            )),
                    )
                    .load::<(i64, SqlUuid)>(reader)?
                    .into_iter()
                    .map(|(idx, id)| (idx, Uuid::from(id)))
                    .collect();

                let images = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(containers::image_id.assume_not_null())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(containers::image_id.assume_not_null()),
                    )
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect();

                let volumes = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_volumes::volume_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_volumes::volume_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_volume| container_volume.map(Uuid::from))
                    .collect();

                let networks = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_networks::network_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_networks::network_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_network| container_network.map(Uuid::from))
                    .collect();

                let device_mappings = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_device_mappings::device_mapping_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_device_mappings::device_mapping_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_device_mapping| container_device_mapping.map(Uuid::from))
                    .collect();

                let device_requests = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_device_requests::device_request_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_device_requests::device_request_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_device_request| container_device_request.map(Uuid::from))
                    .collect();

                let file_binds = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_file_binds::file_bind_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_file_binds::file_bind_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_device_request| container_device_request.map(Uuid::from))
                    .collect();

                let env_files = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select(container_env_files::env_file_id.nullable())
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(container_env_files::env_file_id.nullable()),
                    )
                    .load::<Option<SqlUuid>>(reader)?
                    .into_iter()
                    .filter_map(|container_device_request| container_device_request.map(Uuid::from))
                    .collect();

                Ok(Some(DeploymentResource {
                    containers,
                    images,
                    volumes,
                    networks,
                    device_mappings,
                    device_requests,
                    file_binds,
                    env_files,
                }))
            })
            .await?;

        Ok(deployment)
    }

    /// Fetches the containers for a deployment to be stopped for an update
    #[instrument(skip(self))]
    pub(crate) async fn load_deployment_containers_update_from(
        &self,
        DeploymentUpdate { from, to }: DeploymentUpdate,
    ) -> Result<Option<Vec<SqlUuid>>> {
        let containers = self
            .handle
            .for_read(move |reader| {
                let from = SqlUuid::new(from);
                if !Deployment::exists(&from).get_result(reader)? {
                    return Ok(None);
                }

                let to = SqlUuid::new(to);

                let containers = deployment_containers::table
                    .select(deployment_containers::container_id)
                    .filter(deployment_containers::deployment_id.eq(from))
                    // Exclude container in the update
                    .except(
                        deployment_containers::table
                            .select(deployment_containers::container_id)
                            .filter(deployment_containers::deployment_id.eq(to)),
                    )
                    .load::<SqlUuid>(reader)?;

                Ok(Some(containers))
            })
            .await?;

        Ok(containers)
    }
}

/// Check that a deployment with the given id exists, and there are no missing rows
/// for the various resources
fn is_deployment_complete(
    reader: &mut SqliteConnection,
    id: &SqlUuid,
) -> std::result::Result<bool, HandleError> {
    let has_containers = select(exists(
        deployment_containers::table.filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if !has_containers {
        return Ok(false);
    }

    let has_missing_containers = select(exists(
        deployment_missing_containers::table
            .filter(deployment_missing_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_containers {
        return Ok(false);
    }

    let has_missing_images = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_images::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_images {
        return Ok(false);
    }

    let has_missing_networks = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_networks::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_networks {
        return Ok(false);
    }

    let has_missing_volumes = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_volumes::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_volumes {
        return Ok(false);
    }

    let has_missing_device_mappings = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_device_mappings::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_device_mappings {
        return Ok(false);
    }

    let has_missing_device_requests = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_device_requests::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_device_requests {
        return Ok(false);
    }

    let has_missing_file_binds = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_file_binds::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;
    if has_missing_file_binds {
        return Ok(false);
    }

    let has_missing_env_files = select(exists(
        deployment_containers::table
            .inner_join(containers::table.inner_join(container_missing_env_files::table))
            .filter(deployment_containers::deployment_id.eq(id)),
    ))
    .first::<bool>(reader)?;

    Ok(!has_missing_env_files)
}

impl From<CreateDeployment> for Deployment {
    fn from(CreateDeployment { id, containers: _ }: CreateDeployment) -> Self {
        Self {
            id: SqlUuid::new(id),
            status: DeploymentStatus::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use diesel::OptionalExtension;
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use rustc_hash::FxHashSet;
    use tempfile::TempDir;

    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::env_file::tests::create_env_file_req;
    use crate::requests::file_bind::tests::create_file_bind_req;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;
    use crate::requests::{ReqUuid, VecReqUuid};

    use super::*;

    pub(crate) async fn find_deployment(store: &StateStore, id: Uuid) -> Option<Deployment> {
        store
            .handle
            .for_read(move |reader| {
                Deployment::find_id(&SqlUuid::new(id))
                    .first::<Deployment>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn should_create() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        store.create_image(image).await.unwrap();
        store.create_volume(volume).await.unwrap();
        store.create_network(network).await.unwrap();
        store.create_device_mapping(device_mapping).await.unwrap();
        store.create_device_request(device_request).await.unwrap();
        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();

        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };
        store.create_deployment(deployment).await.unwrap();

        let deployment = find_deployment(&store, deployment_id).await.unwrap();
        let exp = Deployment {
            id: SqlUuid::new(deployment_id),
            status: DeploymentStatus::Received,
        };
        assert_eq!(deployment, exp);

        let containers = store
            .load_deployment_containers(deployment_id)
            .await
            .unwrap()
            .unwrap();
        let exp = vec![SqlUuid::new(container.id.0)];
        assert_eq!(containers, exp);
    }

    #[tokio::test]
    async fn update_status() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let container_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![ReqUuid(container_id)]),
        };
        store.create_deployment(deployment).await.unwrap();

        store
            .update_deployment_status(deployment_id, DeploymentStatus::Stopped)
            .await
            .unwrap();

        let deployment = find_deployment(&store, deployment_id).await.unwrap();
        let exp = Deployment {
            id: SqlUuid::new(deployment_id),
            status: DeploymentStatus::Stopped,
        };
        assert_eq!(deployment, exp);
    }

    #[tokio::test]
    async fn find_complete_deployment() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        store.create_image(image.clone()).await.unwrap();
        store.create_volume(volume.clone()).await.unwrap();
        store.create_network(network.clone()).await.unwrap();
        store
            .create_device_mapping(device_mapping.clone())
            .await
            .unwrap();
        store
            .create_device_request(device_request.clone())
            .await
            .unwrap();
        store.create_file_bind(file_bind.clone()).await.unwrap();
        store.create_env_file(env_file.clone()).await.unwrap();
        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();

        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };
        store.create_deployment(deployment).await.unwrap();

        let deployment = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();
        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container.id.0)]),
            images: FxHashSet::from_iter([image.id.0]),
            volumes: FxHashSet::from_iter([volume.id.0]),
            networks: FxHashSet::from_iter([network.id.0]),
            device_mappings: FxHashSet::from_iter([device_mapping.id.0]),
            device_requests: FxHashSet::from_iter([device_request.id.0]),
            file_binds: FxHashSet::from_iter([file_bind.id.0]),
            env_files: FxHashSet::from_iter([env_file.id.0]),
        };

        assert_eq!(deployment, exp);
    }

    #[tokio::test]
    async fn shared_resources_delete() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id_1 = Uuid::new_v4();

        let image = create_image_req(deployment_id_1);
        let volume = create_volume_req(deployment_id_1);
        let network = create_network_req(deployment_id_1);
        let device_mapping = create_device_mapping_req(deployment_id_1);
        let device_request = create_device_request(deployment_id_1);
        let file_bind = create_file_bind_req(deployment_id_1);
        let env_file = create_env_file_req(deployment_id_1);
        let container_1 = create_container_req(
            deployment_id_1,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        store.create_image(image.clone()).await.unwrap();
        store.create_volume(volume.clone()).await.unwrap();
        store.create_network(network.clone()).await.unwrap();
        store
            .create_device_mapping(device_mapping.clone())
            .await
            .unwrap();
        store
            .create_device_request(device_request.clone())
            .await
            .unwrap();
        store
            .create_container(Box::new(container_1.clone()))
            .await
            .unwrap();

        let deployment_1 = CreateDeployment {
            id: ReqUuid(deployment_id_1),
            containers: VecReqUuid(vec![ReqUuid(container_1.id.0)]),
        };
        store.create_deployment(deployment_1).await.unwrap();

        let deployment_id_2 = Uuid::new_v4();
        let container_2 = create_container_req(
            deployment_id_2,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );
        store
            .create_container(Box::new(container_2.clone()))
            .await
            .unwrap();

        let deployment_2 = CreateDeployment {
            id: ReqUuid(deployment_id_2),
            containers: VecReqUuid(vec![container_2.id]),
        };
        store.create_deployment(deployment_2).await.unwrap();

        let res = store
            .find_deployment_for_delete(deployment_id_1)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container_1.id.0)]),
            images: FxHashSet::default(),
            volumes: FxHashSet::default(),
            networks: FxHashSet::default(),
            device_mappings: FxHashSet::default(),
            device_requests: FxHashSet::default(),
            file_binds: FxHashSet::default(),
            env_files: FxHashSet::default(),
        };

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn sorted_deployment_containers() {
        let tmp = TempDir::with_prefix("sorted_deployment_containers").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);
        let container_1 = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );
        let container_2 = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        store.create_image(image.clone()).await.unwrap();
        store.create_volume(volume.clone()).await.unwrap();
        store.create_network(network.clone()).await.unwrap();
        store
            .create_device_mapping(device_mapping.clone())
            .await
            .unwrap();
        store
            .create_device_request(device_request.clone())
            .await
            .unwrap();
        store.create_file_bind(file_bind.clone()).await.unwrap();
        store.create_env_file(env_file.clone()).await.unwrap();
        store
            .create_container(Box::new(container_1.clone()))
            .await
            .unwrap();
        store
            .create_container(Box::new(container_2.clone()))
            .await
            .unwrap();

        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container_1.id, container_2.id]),
        };
        store.create_deployment(deployment).await.unwrap();

        let res = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container_1.id.0), (1, container_2.id.0)]),
            images: FxHashSet::from_iter([image.id.0]),
            volumes: FxHashSet::from_iter([volume.id.0]),
            networks: FxHashSet::from_iter([network.id.0]),
            device_mappings: FxHashSet::from_iter([device_mapping.id.0]),
            device_requests: FxHashSet::from_iter([device_request.id.0]),
            file_binds: FxHashSet::from_iter([file_bind.id.0]),
            env_files: FxHashSet::from_iter([env_file.id.0]),
        };

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn find_complete_deployment_not_found() {
        let tmp = TempDir::with_prefix("find_complete_deployment_not_found").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let res = store
            .find_complete_deployment(Uuid::new_v4())
            .await
            .unwrap();
        assert_eq!(res, None);
    }

    #[tokio::test]
    async fn find_complete_deployment_incomplete() {
        let tmp = TempDir::with_prefix("find_complete_deployment_incomplete").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        // Missing container
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };
        store.create_deployment(deployment).await.unwrap();
        assert_eq!(
            store.find_complete_deployment(deployment_id).await.unwrap(),
            None
        );

        // Container created, but all resources missing
        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();
        assert_eq!(
            store.find_complete_deployment(deployment_id).await.unwrap(),
            None
        );

        // Create resources one by one; still incomplete until the last one
        store.create_image(image.clone()).await.unwrap();
        store.create_volume(volume.clone()).await.unwrap();
        store.create_network(network.clone()).await.unwrap();
        store
            .create_device_mapping(device_mapping.clone())
            .await
            .unwrap();
        store
            .create_device_request(device_request.clone())
            .await
            .unwrap();
        store.create_file_bind(file_bind.clone()).await.unwrap();
        assert_eq!(
            store.find_complete_deployment(deployment_id).await.unwrap(),
            None
        );

        // Create the final missing resource (env_file)
        store.create_env_file(env_file.clone()).await.unwrap();
        let res = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container.id.0)]),
            images: FxHashSet::from_iter([image.id.0]),
            volumes: FxHashSet::from_iter([volume.id.0]),
            networks: FxHashSet::from_iter([network.id.0]),
            device_mappings: FxHashSet::from_iter([device_mapping.id.0]),
            device_requests: FxHashSet::from_iter([device_request.id.0]),
            file_binds: FxHashSet::from_iter([file_bind.id.0]),
            env_files: FxHashSet::from_iter([env_file.id.0]),
        };
        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn find_complete_deployment_without_optional_resources() {
        let tmp = TempDir::with_prefix("find_complete_deployment_no_opt").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let image = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);
        let mut container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );
        container.network_ids = None;
        container.volume_ids = None;
        container.device_mapping_ids = None;
        container.device_request_ids = None;
        container.file_bind_ids = None;
        container.env_file_ids = None;

        store.create_image(image.clone()).await.unwrap();
        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();

        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };
        store.create_deployment(deployment).await.unwrap();

        let res = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();
        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container.id.0)]),
            images: FxHashSet::from_iter([image.id.0]),
            volumes: FxHashSet::default(),
            networks: FxHashSet::default(),
            device_mappings: FxHashSet::default(),
            device_requests: FxHashSet::default(),
            file_binds: FxHashSet::default(),
            env_files: FxHashSet::default(),
        };
        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn find_complete_deployment_multiple_resources_and_isolation() {
        let tmp = TempDir::with_prefix("find_complete_deployment_multi").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        // Deployment 1: 2 containers with multiple distinct & shared resources
        let deployment_1_id = Uuid::new_v4();
        let image_1 = create_image_req(deployment_1_id);
        let image_2 = create_image_req(deployment_1_id);
        let volume_1 = create_volume_req(deployment_1_id);
        let volume_2 = create_volume_req(deployment_1_id);
        let network_1 = create_network_req(deployment_1_id);
        let network_2 = create_network_req(deployment_1_id);
        let device_mapping_1 = create_device_mapping_req(deployment_1_id);
        let device_mapping_2 = create_device_mapping_req(deployment_1_id);
        let device_request_1 = create_device_request(deployment_1_id);
        let device_request_2 = create_device_request(deployment_1_id);
        let file_bind_1 = create_file_bind_req(deployment_1_id);
        let file_bind_2 = create_file_bind_req(deployment_1_id);
        let env_file_1 = create_env_file_req(deployment_1_id);
        let env_file_2 = create_env_file_req(deployment_1_id);

        let mut container_1 = create_container_req(
            deployment_1_id,
            &image_1,
            &volume_1,
            &network_1,
            &device_mapping_1,
            &device_request_1,
            &file_bind_1,
            &env_file_1,
        );
        container_1.volume_ids = Some(VecReqUuid(vec![volume_1.id, volume_2.id]));
        container_1.network_ids = Some(VecReqUuid(vec![network_1.id, network_2.id]));
        container_1.device_mapping_ids =
            Some(VecReqUuid(vec![device_mapping_1.id, device_mapping_2.id]));
        container_1.device_request_ids =
            Some(VecReqUuid(vec![device_request_1.id, device_request_2.id]));
        container_1.file_bind_ids = Some(VecReqUuid(vec![file_bind_1.id, file_bind_2.id]));
        container_1.env_file_ids = Some(VecReqUuid(vec![env_file_1.id, env_file_2.id]));

        let container_2 = create_container_req(
            deployment_1_id,
            &image_2,
            &volume_2,
            &network_2,
            &device_mapping_2,
            &device_request_2,
            &file_bind_2,
            &env_file_2,
        );

        for image in [&image_1, &image_2] {
            store.create_image(image.clone()).await.unwrap();
        }
        for volume in [&volume_1, &volume_2] {
            store.create_volume(volume.clone()).await.unwrap();
        }
        for network in [&network_1, &network_2] {
            store.create_network(network.clone()).await.unwrap();
        }
        for dm in [&device_mapping_1, &device_mapping_2] {
            store.create_device_mapping(dm.clone()).await.unwrap();
        }
        for dr in [&device_request_1, &device_request_2] {
            store.create_device_request(dr.clone()).await.unwrap();
        }
        for fb in [&file_bind_1, &file_bind_2] {
            store.create_file_bind(fb.clone()).await.unwrap();
        }
        for ef in [&env_file_1, &env_file_2] {
            store.create_env_file(ef.clone()).await.unwrap();
        }
        store
            .create_container(Box::new(container_1.clone()))
            .await
            .unwrap();
        store
            .create_container(Box::new(container_2.clone()))
            .await
            .unwrap();
        store
            .create_deployment(CreateDeployment {
                id: ReqUuid(deployment_1_id),
                containers: VecReqUuid(vec![container_1.id, container_2.id]),
            })
            .await
            .unwrap();

        // Deployment 2: completely separate container & resources
        let deployment_2_id = Uuid::new_v4();
        let other_image = create_image_req(deployment_2_id);
        let other_volume = create_volume_req(deployment_2_id);
        let other_network = create_network_req(deployment_2_id);
        let other_dm = create_device_mapping_req(deployment_2_id);
        let other_dr = create_device_request(deployment_2_id);
        let other_fb = create_file_bind_req(deployment_2_id);
        let other_ef = create_env_file_req(deployment_2_id);
        let other_container = create_container_req(
            deployment_2_id,
            &other_image,
            &other_volume,
            &other_network,
            &other_dm,
            &other_dr,
            &other_fb,
            &other_ef,
        );
        store.create_image(other_image).await.unwrap();
        store.create_volume(other_volume).await.unwrap();
        store.create_network(other_network).await.unwrap();
        store.create_device_mapping(other_dm).await.unwrap();
        store.create_device_request(other_dr).await.unwrap();
        store.create_file_bind(other_fb).await.unwrap();
        store.create_env_file(other_ef).await.unwrap();
        store
            .create_container(Box::new(other_container.clone()))
            .await
            .unwrap();
        store
            .create_deployment(CreateDeployment {
                id: ReqUuid(deployment_2_id),
                containers: VecReqUuid(vec![other_container.id]),
            })
            .await
            .unwrap();

        let res = store
            .find_complete_deployment(deployment_1_id)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container_1.id.0), (1, container_2.id.0)]),
            images: FxHashSet::from_iter([image_1.id.0, image_2.id.0]),
            volumes: FxHashSet::from_iter([volume_1.id.0, volume_2.id.0]),
            networks: FxHashSet::from_iter([network_1.id.0, network_2.id.0]),
            device_mappings: FxHashSet::from_iter([device_mapping_1.id.0, device_mapping_2.id.0]),
            device_requests: FxHashSet::from_iter([device_request_1.id.0, device_request_2.id.0]),
            file_binds: FxHashSet::from_iter([file_bind_1.id.0, file_bind_2.id.0]),
            env_files: FxHashSet::from_iter([env_file_1.id.0, env_file_2.id.0]),
        };
        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn find_complete_deployment_partial_containers() {
        let tmp = TempDir::with_prefix("find_complete_deployment_partial").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();
        let image_1 = create_image_req(deployment_id);
        let image_2 = create_image_req(deployment_id);
        let volume = create_volume_req(deployment_id);
        let network = create_network_req(deployment_id);
        let device_mapping = create_device_mapping_req(deployment_id);
        let device_request = create_device_request(deployment_id);
        let file_bind = create_file_bind_req(deployment_id);
        let env_file = create_env_file_req(deployment_id);

        let container_1 = create_container_req(
            deployment_id,
            &image_1,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );
        let container_2 = create_container_req(
            deployment_id,
            &image_2,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        // Create all resources for container_1, but leave image_2 missing for container_2
        store.create_image(image_1.clone()).await.unwrap();
        store.create_volume(volume.clone()).await.unwrap();
        store.create_network(network.clone()).await.unwrap();
        store
            .create_device_mapping(device_mapping.clone())
            .await
            .unwrap();
        store
            .create_device_request(device_request.clone())
            .await
            .unwrap();
        store.create_file_bind(file_bind.clone()).await.unwrap();
        store.create_env_file(env_file.clone()).await.unwrap();
        store
            .create_container(Box::new(container_1.clone()))
            .await
            .unwrap();
        store
            .create_container(Box::new(container_2.clone()))
            .await
            .unwrap();

        store
            .create_deployment(CreateDeployment {
                id: ReqUuid(deployment_id),
                containers: VecReqUuid(vec![container_1.id, container_2.id]),
            })
            .await
            .unwrap();

        // Container 1 is complete, container 2 is missing image_2 -> deployment is incomplete
        assert_eq!(
            store.find_complete_deployment(deployment_id).await.unwrap(),
            None
        );

        // Once image_2 is created, the deployment becomes complete
        store.create_image(image_2.clone()).await.unwrap();
        let res = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: BTreeMap::from_iter([(0, container_1.id.0), (1, container_2.id.0)]),
            images: FxHashSet::from_iter([image_1.id.0, image_2.id.0]),
            volumes: FxHashSet::from_iter([volume.id.0]),
            networks: FxHashSet::from_iter([network.id.0]),
            device_mappings: FxHashSet::from_iter([device_mapping.id.0]),
            device_requests: FxHashSet::from_iter([device_request.id.0]),
            file_binds: FxHashSet::from_iter([file_bind.id.0]),
            env_files: FxHashSet::from_iter([env_file.id.0]),
        };
        assert_eq!(res, exp);
    }
}
