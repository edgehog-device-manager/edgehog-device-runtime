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

use diesel::dsl::exists;
use diesel::{
    delete, insert_or_ignore_into, select, update, ExpressionMethods, NullableExpressionMethods,
    OptionalExtension, QueryDsl, RunQueryDsl, SelectableHelper, SqliteConnection,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::container::{Container, ContainerNetwork, ContainerVolume};
use edgehog_store::models::containers::deployment::{
    Deployment, DeploymentContainer, DeploymentMissingContainer, DeploymentStatus,
};
use edgehog_store::models::QueryModel;
use edgehog_store::schema::containers::{
    container_missing_images, container_missing_networks, container_missing_volumes,
    container_networks, container_volumes, containers, deployment_containers,
    deployment_missing_containers, deployments,
};
use itertools::Itertools;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::requests::deployment::CreateDeployment;
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

    /// Updates the status of a deployment
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

    /// Fetches an deployment by id
    #[instrument(skip(self))]
    pub(crate) async fn deployment(&mut self, id: Uuid) -> Result<Option<Deployment>> {
        let deployment = self
            .handle
            .for_read(move |reader| {
                let deployment: Option<Deployment> = Deployment::find_id(&SqlUuid::new(id))
                    .first(reader)
                    .optional()?;

                Ok(deployment)
            })
            .await?;

        Ok(deployment)
    }

    #[instrument(skip(self))]
    pub(crate) async fn deployments_in(
        &mut self,
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

    /// Fetches an deployment by id
    #[instrument(skip(self))]
    pub(crate) async fn deployment_containers(&mut self, id: Uuid) -> Result<Option<Vec<Uuid>>> {
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
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(Uuid::from)
                    .collect_vec();

                Ok(Some(containers))
            })
            .await?;

        Ok(containers)
    }

    pub(crate) async fn complete_deployment(
        &mut self,
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

                deployment_resource(reader, &id).map(Some)
            })
            .await?;

        Ok(deployment)
    }

    pub(crate) async fn deployment_resource(
        &mut self,
        id: Uuid,
    ) -> Result<Option<DeploymentResource>> {
        let deployment = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);

                if !Deployment::exists(&id).get_result(reader)? {
                    return Ok(None);
                }

                deployment_resource(reader, &id).map(Some)
            })
            .await?;

        Ok(deployment)
    }
}

fn deployment_resource(
    reader: &mut SqliteConnection,
    id: &SqlUuid,
) -> std::result::Result<DeploymentResource, HandleError> {
    let values = deployment_containers::table
        .inner_join(
            // Join the container related tables
            containers::table
                .left_join(container_networks::table)
                .left_join(container_volumes::table),
        )
        .select((
            deployment_containers::container_id,
            containers::image_id.assume_not_null(),
            Option::<ContainerNetwork>::as_select(),
            Option::<ContainerVolume>::as_select(),
        ))
        .filter(deployment_containers::deployment_id.eq(id))
        .filter(containers::image_id.is_not_null())
        .load::<(
            SqlUuid,
            SqlUuid,
            Option<ContainerNetwork>,
            Option<ContainerVolume>,
        )>(reader)?;

    let resource = values.into_iter().fold(
        DeploymentResource::default(),
        |mut acc, (container_id, image_id, c_network, c_volume)| {
            acc.containers.insert(*container_id);
            acc.images.insert(*image_id);

            if let Some(c_network) = c_network {
                acc.networks.insert(*c_network.network_id);
            }

            if let Some(c_volume) = c_volume {
                acc.volumes.insert(*c_volume.volume_id);
            }
            acc
        },
    );

    Ok(resource)
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
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::{
        container::CreateContainer, image::tests::mock_image_req, network::CreateNetwork,
        volume::CreateVolume, ReqUuid, VecReqUuid,
    };

    use super::*;

    #[tokio::test]
    async fn should_create() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let image_id = Uuid::new_v4();
        let image = mock_image_req(image_id, "postgres:15", "");
        store.create_image(image).await.unwrap();

        let volume_id = ReqUuid(Uuid::new_v4());
        let volume = CreateVolume {
            id: volume_id,
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let network_id = ReqUuid(Uuid::new_v4());
        let network = CreateNetwork {
            id: network_id,
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let container_id = Uuid::new_v4();
        let container = CreateContainer {
            id: ReqUuid(container_id),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            image: "postgres:15".to_string(),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            privileged: false,
        };
        store.create_container(container).await.unwrap();

        let deployment_id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![ReqUuid(container_id)]),
        };
        store.create_deployment(deployment).await.unwrap();

        let deployment = store.deployment(deployment_id).await.unwrap();
        let exp = Deployment {
            id: SqlUuid::new(deployment_id),
            status: DeploymentStatus::Received,
        };
        assert_eq!(deployment, Some(exp));

        let containers = store.deployment_containers(deployment_id).await.unwrap();
        let exp = vec![DeploymentContainer {
            deployment_id: SqlUuid::new(deployment_id),
            container_id: SqlUuid::new(container_id),
        }];
        assert_eq!(containers, exp);
    }

    #[tokio::test]
    async fn update_status() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

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

        let deployment = store.deployment(deployment_id).await.unwrap();
        let exp = Deployment {
            id: SqlUuid::new(deployment_id),
            status: DeploymentStatus::Stopped,
        };
        assert_eq!(deployment, Some(exp));
    }
}
