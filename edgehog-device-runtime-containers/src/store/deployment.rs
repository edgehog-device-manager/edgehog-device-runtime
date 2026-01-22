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

use diesel::dsl::exists;
use diesel::{
    CombineDsl, ExpressionMethods, NullableExpressionMethods, QueryDsl, RunQueryDsl,
    SelectableHelper, SqliteConnection, delete, insert_or_ignore_into, select, update,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::container::{
    Container, ContainerDeviceMapping, ContainerNetwork, ContainerVolume,
};
use edgehog_store::models::containers::deployment::{
    Deployment, DeploymentContainer, DeploymentMissingContainer, DeploymentStatus,
};
use edgehog_store::schema::containers::{
    container_device_mappings, container_missing_images, container_missing_networks,
    container_missing_volumes, container_networks, container_volumes, containers,
    deployment_containers, deployment_missing_containers, deployments,
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

                for container_id in containers {
                    let exists: bool = Container::exists(&container_id).get_result(writer)?;

                    if !exists {
                        insert_or_ignore_into(deployment_missing_containers::table)
                            .values(DeploymentMissingContainer {
                                deployment_id: deployment.id,
                                container_id,
                            })
                            .execute(writer)?;

                        continue;
                    }

                    insert_or_ignore_into(deployment_containers::table)
                        .values(DeploymentContainer {
                            deployment_id: deployment.id,
                            container_id,
                        })
                        .execute(writer)?;
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

                let rows = Deployment::join_resources()
                    .filter(deployment_containers::deployment_id.eq(id))
                    .select((
                        deployment_containers::container_id,
                        containers::image_id.assume_not_null(),
                        Option::<ContainerNetwork>::as_select(),
                        Option::<ContainerVolume>::as_select(),
                        Option::<ContainerDeviceMapping>::as_select(),
                    ))
                    .load::<(
                        SqlUuid,
                        SqlUuid,
                        Option<ContainerNetwork>,
                        Option<ContainerVolume>,
                        Option<ContainerDeviceMapping>,
                    )>(reader)?;

                Ok(Some(DeploymentResource::from(rows)))
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
                    .select(deployment_containers::container_id)
                    .except(
                        Deployment::join_resources()
                            .filter(deployment_containers::deployment_id.ne(id))
                            .select(deployment_containers::container_id),
                    )
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
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

                let device_mapping = Deployment::join_resources()
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

                Ok(Some(DeploymentResource {
                    containers,
                    images,
                    volumes,
                    networks,
                    device_mapping,
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
    select(exists(
        deployments::table
            .left_join(deployment_missing_containers::table)
            .inner_join(
                deployment_containers::table.inner_join(
                    containers::table
                        .left_join(container_missing_images::table)
                        .left_join(container_missing_networks::table)
                        .left_join(container_missing_volumes::table),
                ),
            )
            .filter(deployments::id.eq(id))
            .filter(deployment_missing_containers::deployment_id.is_null())
            .filter(container_missing_images::container_id.is_null())
            .filter(container_missing_networks::container_id.is_null())
            .filter(container_missing_volumes::container_id.is_null()),
    ))
    .first::<bool>(reader)
    .map_err(HandleError::Query)
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
    use std::collections::HashSet;

    use diesel::OptionalExtension;
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::OptString;
    use crate::requests::device_mapping::CreateDeviceMapping;
    use crate::requests::{
        ReqUuid, VecReqUuid, container::CreateContainer, image::CreateImage,
        network::CreateNetwork, volume::CreateVolume,
    };

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

        let image_id = Uuid::new_v4();
        let image = CreateImage {
            id: ReqUuid(image_id),
            deployment_id: ReqUuid(deployment_id),
            reference: "postgres:15".to_string(),
            registry_auth: String::new(),
        };
        store.create_image(image).await.unwrap();

        let volume_id = ReqUuid(Uuid::new_v4());
        let volume = CreateVolume {
            id: volume_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let network_id = ReqUuid(Uuid::new_v4());
        let network = CreateNetwork {
            id: network_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let device_mapping_id = ReqUuid(Uuid::new_v4());
        let device_mapping = CreateDeviceMapping {
            id: device_mapping_id,
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "dev/tty12".to_string(),
            c_group_permissions: OptString::from("msv".to_string()),
        };
        store.create_device_mapping(device_mapping).await.unwrap();

        let container_id = Uuid::new_v4();
        let container = CreateContainer {
            id: ReqUuid(container_id),
            deployment_id: ReqUuid(deployment_id),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            device_mapping_ids: VecReqUuid(vec![device_mapping_id]),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            cap_add: vec!["CAP_CHOWN".to_string()],
            cap_drop: vec!["CAP_KILL".to_string()],
            cpu_period: 1000,
            cpu_quota: 100,
            cpu_realtime_period: 1000,
            cpu_realtime_runtime: 100,
            memory: 4096,
            memory_reservation: 1024,
            memory_swap: 8192,
            memory_swappiness: 50,
            volume_driver: "local".to_string().into(),
            storage_opt: vec!["size=1024k".to_string()],
            read_only_rootfs: true,
            tmpfs: vec!["/run=rw,noexec,nosuid,size=65536k".to_string()],
            privileged: false,
        };
        store.create_container(Box::new(container)).await.unwrap();

        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![ReqUuid(container_id)]),
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
        let exp = vec![SqlUuid::new(container_id)];
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

        let image_id = Uuid::new_v4();
        let image = CreateImage {
            id: ReqUuid(image_id),
            deployment_id: ReqUuid(deployment_id),
            reference: "postgres:15".to_string(),
            registry_auth: String::new(),
        };
        store.create_image(image).await.unwrap();

        let volume_id = ReqUuid(Uuid::new_v4());
        let volume = CreateVolume {
            id: volume_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let network_id = ReqUuid(Uuid::new_v4());
        let network = CreateNetwork {
            id: network_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let device_mapping_id = ReqUuid(Uuid::new_v4());
        let device_mapping = CreateDeviceMapping {
            id: device_mapping_id,
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "dev/tty12".to_string(),
            c_group_permissions: OptString::from("msv".to_string()),
        };
        store.create_device_mapping(device_mapping).await.unwrap();

        let container_id = Uuid::new_v4();
        let container = CreateContainer {
            id: ReqUuid(container_id),
            deployment_id: ReqUuid(deployment_id),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            device_mapping_ids: VecReqUuid(vec![device_mapping_id]),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            cap_add: vec!["CAP_CHOWN".to_string()],
            cap_drop: vec!["CAP_KILL".to_string()],
            cpu_period: 1000,
            cpu_quota: 100,
            cpu_realtime_period: 1000,
            cpu_realtime_runtime: 100,
            memory: 4096,
            memory_reservation: 1024,
            memory_swap: 8192,
            memory_swappiness: 50,
            volume_driver: "local".to_string().into(),
            storage_opt: vec!["size=1024k".to_string()],
            read_only_rootfs: true,
            tmpfs: vec!["/run=rw,noexec,nosuid,size=65536k".to_string()],
            privileged: false,
        };
        store.create_container(Box::new(container)).await.unwrap();

        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![ReqUuid(container_id)]),
        };
        store.create_deployment(deployment).await.unwrap();

        let deployment = store
            .find_complete_deployment(deployment_id)
            .await
            .unwrap()
            .unwrap();
        let exp = DeploymentResource {
            containers: HashSet::from_iter([container_id]),
            images: HashSet::from_iter([image_id]),
            volumes: HashSet::from_iter([volume_id.0]),
            networks: HashSet::from_iter([network_id.0]),
            device_mapping: HashSet::from_iter([device_mapping_id.0]),
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

        let image_id = Uuid::new_v4();
        let image = CreateImage {
            id: ReqUuid(image_id),
            deployment_id: ReqUuid(deployment_id_1),
            reference: "postgres:15".to_string(),
            registry_auth: String::new(),
        };
        store.create_image(image).await.unwrap();

        let volume_id = ReqUuid(Uuid::new_v4());
        let volume = CreateVolume {
            id: volume_id,
            deployment_id: ReqUuid(deployment_id_1),
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let network_id = ReqUuid(Uuid::new_v4());
        let network = CreateNetwork {
            id: network_id,
            deployment_id: ReqUuid(deployment_id_1),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let device_mapping_id = ReqUuid(Uuid::new_v4());
        let device_mapping = CreateDeviceMapping {
            id: device_mapping_id,
            deployment_id: ReqUuid(deployment_id_1),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "dev/tty12".to_string(),
            c_group_permissions: OptString::from("msv".to_string()),
        };
        store.create_device_mapping(device_mapping).await.unwrap();

        let container_id_1 = Uuid::new_v4();
        let container_1 = CreateContainer {
            id: ReqUuid(container_id_1),
            deployment_id: ReqUuid(deployment_id_1),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            device_mapping_ids: VecReqUuid(vec![device_mapping_id]),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            cap_add: vec!["CAP_CHOWN".to_string()],
            cap_drop: vec!["CAP_KILL".to_string()],
            cpu_period: 1000,
            cpu_quota: 100,
            cpu_realtime_period: 1000,
            cpu_realtime_runtime: 100,
            memory: 4096,
            memory_reservation: 1024,
            memory_swap: 8192,
            memory_swappiness: 50,
            volume_driver: "local".to_string().into(),
            storage_opt: vec!["size=1024k".to_string()],
            read_only_rootfs: true,
            tmpfs: vec!["/run=rw,noexec,nosuid,size=65536k".to_string()],
            privileged: false,
        };
        store.create_container(Box::new(container_1)).await.unwrap();

        let deployment_1 = CreateDeployment {
            id: ReqUuid(deployment_id_1),
            containers: VecReqUuid(vec![ReqUuid(container_id_1)]),
        };
        store.create_deployment(deployment_1).await.unwrap();

        let deployment_id_2 = Uuid::new_v4();
        let container_id_2 = Uuid::new_v4();
        let container_2 = CreateContainer {
            id: ReqUuid(container_id_2),
            deployment_id: ReqUuid(deployment_id_2),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            device_mapping_ids: VecReqUuid(vec![device_mapping_id]),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            cap_add: vec!["CAP_CHOWN".to_string()],
            cap_drop: vec!["CAP_KILL".to_string()],
            cpu_period: 1000,
            cpu_quota: 100,
            cpu_realtime_period: 1000,
            cpu_realtime_runtime: 100,
            memory: 4096,
            memory_reservation: 1024,
            memory_swap: 8192,
            memory_swappiness: 50,
            volume_driver: "local".to_string().into(),
            storage_opt: vec!["size=1024k".to_string()],
            read_only_rootfs: true,
            tmpfs: vec!["/run=rw,noexec,nosuid,size=65536k".to_string()],
            privileged: false,
        };
        store.create_container(Box::new(container_2)).await.unwrap();

        let deployment_2 = CreateDeployment {
            id: ReqUuid(deployment_id_2),
            containers: VecReqUuid(vec![ReqUuid(container_id_2)]),
        };
        store.create_deployment(deployment_2).await.unwrap();

        let res = store
            .find_deployment_for_delete(deployment_id_1)
            .await
            .unwrap()
            .unwrap();

        let exp = DeploymentResource {
            containers: HashSet::from_iter([container_id_1]),
            images: HashSet::new(),
            volumes: HashSet::new(),
            networks: HashSet::new(),
            device_mapping: HashSet::new(),
        };

        assert_eq!(res, exp);
    }
}
