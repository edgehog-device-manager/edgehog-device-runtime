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
use std::ops::Not;
use std::str::FromStr;

use diesel::dsl::exists;
use diesel::query_dsl::methods::{FilterDsl, SelectDsl};
use diesel::{
    delete, insert_or_ignore_into, select, update, ExpressionMethods, OptionalExtension,
    RunQueryDsl, SqliteConnection,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::container::{
    ContainerBind, ContainerEnv, ContainerNetwork, ContainerPortBind, ContainerRestartPolicy,
    ContainerStatus, ContainerVolume, HostPort,
};
use edgehog_store::models::containers::deployment::DeploymentMissingContainer;
use edgehog_store::models::QueryModel;
use edgehog_store::schema::containers::{deployment_containers, images};
use edgehog_store::{
    models::containers::{
        container::{
            Container, ContainerMissingImage, ContainerMissingNetwork, ContainerMissingVolume,
        },
        image::Image,
        network::Network,
        volume::Volume,
    },
    schema::containers::{
        container_binds, container_env, container_missing_images, container_missing_networks,
        container_missing_volumes, container_networks, container_port_bindings, container_volumes,
        containers,
    },
};
use itertools::Itertools;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::container::{Binding, PortBindingMap};
use crate::docker::container::Container as ContainerResource;
use crate::requests::container::{
    parse_port_binding, CreateContainer, RestartPolicy, RestartPolicyError,
};
use crate::requests::BindingError;

use super::{Result, StateStore};

impl StateStore {
    /// Stores the container received from the CreateRequest
    #[instrument(skip_all, fields(id = %container.id))]
    pub(crate) async fn create_container(&self, mut container: CreateContainer) -> Result<()> {
        let image_id = SqlUuid::new(container.image_id);
        let networks = container.network_ids.iter().map(SqlUuid::new).collect_vec();
        let volumes = container.volume_ids.iter().map(SqlUuid::new).collect_vec();

        let envs = Vec::<ContainerEnv>::from(&mut container);
        let binds = Vec::<ContainerBind>::from(&mut container);
        let port_bindings = Vec::<ContainerPortBind>::try_from(&container)?;

        let mut container = Container::try_from(container)?;

        self.handle
            .for_write(move |writer| {
                let image_exists: bool = Image::exists(&image_id).get_result(writer)?;

                if !image_exists {
                    debug!("image is missing, storing image_id into container_missing_images");

                    container.image_id.take();
                }

                insert_or_ignore_into(containers::table)
                    .values(&container)
                    .execute(writer)?;

                if !image_exists {
                    insert_or_ignore_into(container_missing_images::table)
                        .values(ContainerMissingImage {
                            container_id: container.id,
                            image_id,
                        })
                        .execute(writer)?;
                }

                insert_or_ignore_into(container_env::table)
                    .values(envs)
                    .execute(writer)?;

                insert_or_ignore_into(container_binds::table)
                    .values(binds)
                    .execute(writer)?;

                insert_or_ignore_into(container_port_bindings::table)
                    .values(port_bindings)
                    .execute(writer)?;

                for network_id in networks {
                    let network_exists: bool = Network::exists(&network_id).get_result(writer)?;

                    if !network_exists {
                        insert_or_ignore_into(container_missing_networks::table)
                            .values(ContainerMissingNetwork {
                                container_id: container.id,
                                network_id,
                            })
                            .execute(writer)?;

                        continue;
                    }

                    insert_or_ignore_into(container_networks::table)
                        .values(ContainerNetwork {
                            container_id: container.id,
                            network_id,
                        })
                        .execute(writer)?;
                }

                for volume_id in volumes {
                    let volume_exists: bool = Volume::exists(&volume_id).get_result(writer)?;

                    if !volume_exists {
                        insert_or_ignore_into(container_missing_volumes::table)
                            .values(ContainerMissingVolume {
                                container_id: container.id,
                                volume_id,
                            })
                            .execute(writer)?;

                        continue;
                    }

                    insert_or_ignore_into(container_volumes::table)
                        .values(ContainerVolume {
                            container_id: container.id,
                            volume_id,
                        })
                        .execute(writer)?;
                }

                // Update deployment missing container
                insert_or_ignore_into(deployment_containers::table)
                    .values(DeploymentMissingContainer::find_by_container(&container.id))
                    .execute(writer)?;

                delete(DeploymentMissingContainer::find_by_container(&container.id))
                    .execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the status of a container
    #[instrument(skip(self))]
    pub(crate) async fn update_container_status(
        &self,
        id: Uuid,
        status: ContainerStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Container::find_id(&SqlUuid::new(id)))
                    .set(containers::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the [`Container`] local id.
    #[instrument(skip(self))]
    pub(crate) async fn update_container_local_id(
        &self,
        id: Uuid,
        local_id: Option<String>,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Container::find_id(&SqlUuid::new(id)))
                    .set(containers::local_id.eq(local_id))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes the [`Container`] with the given id.
    #[instrument(skip(self))]
    pub(crate) async fn delete_container(&self, id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(Container::find_id(&SqlUuid::new(id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_containers_to_publish(&mut self) -> Result<Vec<SqlUuid>> {
        let container = self
            .handle
            .for_read(move |reader| {
                let container = containers::table
                    .select(containers::id)
                    .filter(containers::status.eq(ContainerStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(container)
            })
            .await?;

        Ok(container)
    }

    /// Fetches an container by id, only if all the resources are present
    #[instrument(skip(self))]
    pub(crate) async fn find_container(&mut self, id: Uuid) -> Result<Option<ContainerResource>> {
        let container = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);
                let Some(container) = Container::find_id(&id)
                    .first::<Container>(reader)
                    .optional()?
                else {
                    debug!("container not found");

                    return Ok(None);
                };

                // Image is missing, we cannot continue
                let Some(image_id) = container.image_id else {
                    debug!("container is missing the image");

                    return Ok(None);
                };
                debug_assert!(!has_missing_networks(reader, &id).unwrap());
                debug_assert!(!has_missing_volumes(reader, &id).unwrap());

                // Error if not found (foreign key constraint is broken).
                let image = images::table
                    .select(images::reference)
                    .filter(images::id.eq(image_id))
                    .first::<String>(reader)?;
                let networks = container_networks::table
                    .select(container_networks::network_id)
                    .filter(container_networks::container_id.eq(&id))
                    .load::<SqlUuid>(reader)?
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let env = container_env::table
                    .select(container_env::value)
                    .filter(container_env::container_id.eq(&id))
                    .load::<String>(reader)?;
                let binds = container_binds::table
                    .select(container_binds::value)
                    .filter(container_binds::container_id.eq(&id))
                    .load::<String>(reader)?;
                let port_bindings = container_port_bindings::table
                    .filter(container_port_bindings::container_id.eq(&id))
                    .load::<ContainerPortBind>(reader)?
                    .into_iter()
                    .fold(HashMap::<String, Vec<Binding>>::new(), |mut acc, p_bind| {
                        acc.entry(p_bind.port).or_default().push(Binding {
                            host_ip: p_bind.host_ip,
                            host_port: p_bind.host_port.map(|h| h.0),
                        });
                        acc
                    });

                Ok(Some(ContainerResource {
                    id: container.local_id,
                    name: container.id.to_string(),
                    image,
                    network_mode: container.network_mode,
                    networks,
                    hostname: container.hostname,
                    restart_policy: container.restart_policy.into(),
                    env,
                    binds,
                    port_bindings: PortBindingMap(port_bindings),
                    privileged: container.privileged,
                }))
            })
            .await?;

        Ok(container)
    }
}

fn has_missing_networks(
    connection: &mut SqliteConnection,
    container_id: &SqlUuid,
) -> std::result::Result<bool, HandleError> {
    select(exists(container_missing_networks::table.filter(
        container_missing_networks::container_id.eq(container_id),
    )))
    .get_result::<bool>(connection)
    .map_err(HandleError::from)
}

fn has_missing_volumes(
    connection: &mut SqliteConnection,
    container_id: &SqlUuid,
) -> std::result::Result<bool, HandleError> {
    select(exists(container_missing_volumes::table.filter(
        container_missing_volumes::container_id.eq(container_id),
    )))
    .get_result::<bool>(connection)
    .map_err(HandleError::from)
}

impl TryFrom<CreateContainer> for Container {
    type Error = RestartPolicyError;

    fn try_from(
        CreateContainer {
            id,
            image_id,
            hostname,
            restart_policy,
            network_mode,
            privileged,
            network_ids: _,
            volume_ids: _,
            env: _,
            binds: _,
            port_bindings: _,
        }: CreateContainer,
    ) -> std::result::Result<Self, Self::Error> {
        let restart_policy = RestartPolicy::from_str(&restart_policy)?;
        let restart_policy = ContainerRestartPolicy::from(restart_policy);
        let hostname = hostname.is_empty().not().then_some(hostname);

        Ok(Self {
            id: SqlUuid::new(id),
            local_id: None,
            image_id: Some(SqlUuid::new(image_id)),
            status: ContainerStatus::default(),
            network_mode,
            hostname,
            restart_policy,
            privileged,
        })
    }
}

impl From<RestartPolicy> for ContainerRestartPolicy {
    fn from(value: RestartPolicy) -> Self {
        match value {
            RestartPolicy::Empty => ContainerRestartPolicy::Empty,
            RestartPolicy::No => ContainerRestartPolicy::No,
            RestartPolicy::Always => ContainerRestartPolicy::Always,
            RestartPolicy::UnlessStopped => ContainerRestartPolicy::UnlessStopped,
            RestartPolicy::OnFailure => ContainerRestartPolicy::OnFailure,
        }
    }
}

impl From<ContainerRestartPolicy> for RestartPolicy {
    fn from(value: ContainerRestartPolicy) -> Self {
        match value {
            ContainerRestartPolicy::Empty => RestartPolicy::Empty,
            ContainerRestartPolicy::No => RestartPolicy::No,
            ContainerRestartPolicy::Always => RestartPolicy::Always,
            ContainerRestartPolicy::UnlessStopped => RestartPolicy::UnlessStopped,
            ContainerRestartPolicy::OnFailure => RestartPolicy::OnFailure,
        }
    }
}

impl From<&mut CreateContainer> for Vec<ContainerEnv> {
    fn from(value: &mut CreateContainer) -> Self {
        let container_id = SqlUuid::new(value.id);

        std::mem::take(&mut value.env)
            .into_iter()
            .map(|value| ContainerEnv {
                container_id,
                value,
            })
            .collect_vec()
    }
}

impl From<&mut CreateContainer> for Vec<ContainerBind> {
    fn from(value: &mut CreateContainer) -> Self {
        let container_id = SqlUuid::new(value.id);

        std::mem::take(&mut value.binds)
            .into_iter()
            .map(|value| ContainerBind {
                container_id,
                value,
            })
            .collect_vec()
    }
}

impl TryFrom<&CreateContainer> for Vec<ContainerPortBind> {
    type Error = BindingError;

    fn try_from(value: &CreateContainer) -> std::result::Result<Self, Self::Error> {
        let container_id = SqlUuid::new(value.id);

        value
            .port_bindings
            .iter()
            .map(|s| {
                parse_port_binding(s).map(|bind| ContainerPortBind {
                    container_id,
                    port: bind.id(),
                    host_ip: bind.host.host_ip.map(str::to_string),
                    host_port: bind.host.host_port.map(HostPort),
                })
            })
            .try_collect()
    }
}

#[cfg(test)]
mod tests {
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::{
        image::tests::mock_image_req, network::CreateNetwork, volume::CreateVolume, ReqUuid,
        VecReqUuid,
    };

    use super::*;

    async fn find_container(store: &mut StateStore, id: Uuid) -> Option<Container> {
        store
            .handle
            .for_read(move |reader| {
                Container::find_id(&SqlUuid::new(id))
                    .first::<Container>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    impl StateStore {
        pub(crate) async fn container_env(
            &mut self,
            container_id: Uuid,
        ) -> Result<Vec<ContainerEnv>> {
            let env = self
                .handle
                .for_read(move |reader| {
                    let env: Vec<ContainerEnv> = container_env::table
                        .filter(container_env::container_id.eq(SqlUuid::new(container_id)))
                        .load(reader)?;

                    Ok(env)
                })
                .await?;

            Ok(env)
        }

        pub(crate) async fn container_binds(
            &mut self,
            container_id: Uuid,
        ) -> Result<Vec<ContainerBind>> {
            let binds = self
                .handle
                .for_read(move |reader| {
                    let env: Vec<ContainerBind> = container_binds::table
                        .filter(container_binds::container_id.eq(SqlUuid::new(container_id)))
                        .load(reader)?;

                    Ok(env)
                })
                .await?;

            Ok(binds)
        }

        pub(crate) async fn container_port_binds(
            &mut self,
            container_id: Uuid,
        ) -> Result<Vec<ContainerPortBind>> {
            let port_binds = self
                .handle
                .for_read(move |reader| {
                    let port_binds: Vec<ContainerPortBind> = container_port_bindings::table
                        .filter(
                            container_port_bindings::container_id.eq(SqlUuid::new(container_id)),
                        )
                        .load(reader)?;

                    Ok(port_binds)
                })
                .await?;

            Ok(port_binds)
        }
    }

    #[tokio::test]
    async fn should_create_full() {
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
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres:/data".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            privileged: false,
        };
        store.create_container(container).await.unwrap();

        let container = find_container(&mut store, container_id).await.unwrap();
        let exp = Container {
            id: SqlUuid::new(container_id),
            local_id: None,
            image_id: Some(SqlUuid::new(image_id)),
            status: ContainerStatus::Received,
            network_mode: "bridge".to_string(),
            hostname: Some("database".to_string()),
            restart_policy: ContainerRestartPolicy::UnlessStopped,
            privileged: false,
        };
        assert_eq!(container, exp);

        let mut env = store.container_env(container_id).await.unwrap();
        env.sort_unstable();
        let mut exp = vec![
            ContainerEnv {
                container_id: SqlUuid::new(container_id),
                value: "POSTGRES_USER=user".to_string(),
            },
            ContainerEnv {
                container_id: SqlUuid::new(container_id),
                value: "POSTGRES_PASSWORD=password".to_string(),
            },
        ];
        exp.sort_unstable();
        assert_eq!(env, exp);

        let binds = store.container_binds(container_id).await.unwrap();
        let exp = vec![ContainerBind {
            container_id: SqlUuid::new(container_id),
            value: "/var/lib/postgres:/data".to_string(),
        }];
        assert_eq!(binds, exp);

        let port_binds = store.container_port_binds(container_id).await.unwrap();
        let exp = vec![ContainerPortBind {
            container_id: SqlUuid::new(container_id),
            port: "5432/tcp".to_string(),
            host_ip: None,
            host_port: Some(HostPort(5432)),
        }];
        assert_eq!(port_binds, exp);
    }

    #[tokio::test]
    async fn update_status() {
        let tmp = TempDir::with_prefix("update_status").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let container_id = Uuid::new_v4();
        let image_id = Uuid::new_v4();
        let network_id = Uuid::new_v4();
        let volume_id = Uuid::new_v4();
        let container = CreateContainer {
            id: ReqUuid(container_id),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![ReqUuid(network_id)]),
            volume_ids: VecReqUuid(vec![ReqUuid(volume_id)]),
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

        store
            .update_container_status(container_id, ContainerStatus::Published)
            .await
            .unwrap();

        let container = find_container(&mut store, container_id).await.unwrap();
        let exp = Container {
            id: SqlUuid::new(container_id),
            local_id: None,
            image_id: None,
            status: ContainerStatus::Published,
            network_mode: "bridge".to_string(),
            hostname: Some("database".to_string()),
            restart_policy: ContainerRestartPolicy::UnlessStopped,
            privileged: false,
        };
        assert_eq!(container, exp);
    }
}
