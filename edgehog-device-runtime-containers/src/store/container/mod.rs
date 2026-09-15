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

use std::collections::HashMap;

use diesel::dsl::exists;
use diesel::{
    ExpressionMethods, HasQuery, OptionalExtension, QueryDsl, QueryResult, RunQueryDsl,
    SelectableHelper, SqliteConnection, delete, select, update,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::container::{
    Container, ContainerBlkioDeviceReadBps, ContainerBlkioDeviceReadIops,
    ContainerBlkioDeviceWriteBps, ContainerBlkioDeviceWriteIops, ContainerBlkioWeightDevice,
    ContainerPortBindData,
};
use edgehog_store::models::containers::container::{ContainerStatus, ContainerUlimit};
use edgehog_store::models::containers::device_mapping::DeviceMapping;
use edgehog_store::models::containers::device_request::DeviceRequest;
use edgehog_store::schema::containers::{
    container_add_capabilities, container_binds, container_blkio_device_read_bps,
    container_blkio_device_read_iops, container_blkio_device_write_bps,
    container_blkio_device_write_iops, container_blkio_weight_device, container_cmds,
    container_device_cgroup_rules, container_device_mappings, container_device_requests,
    container_dns, container_dns_options, container_dns_search, container_drop_capabilities,
    container_entrypoints, container_env, container_exposed_ports, container_extra_hosts,
    container_group_add, container_healthcheck_test, container_labels, container_log_config,
    container_masked_paths, container_missing_device_mappings, container_missing_device_requests,
    container_missing_networks, container_missing_volumes, container_networks,
    container_port_bindings, container_readonly_paths, container_securityopts,
    container_storage_options, container_sysctls, container_tmpfs, container_ulimits, containers,
    device_requests, images,
};
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::docker::container::Container as ContainerResource;
use crate::store::device_request::load_stored_device_request;

use super::{Result, StateStore};

pub(crate) mod create;

impl StateStore {
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
    pub(crate) async fn load_containers_to_publish(&self) -> Result<Vec<SqlUuid>> {
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

    #[instrument(skip(self))]
    pub(crate) async fn load_containers_in_state(
        &self,
        state: Vec<ContainerStatus>,
    ) -> Result<Vec<(SqlUuid, Option<String>)>> {
        let container = self
            .handle
            .for_read(move |reader| {
                let container = containers::table
                    .select((containers::id, containers::local_id))
                    .filter(containers::status.eq_any(state))
                    .load::<(SqlUuid, Option<String>)>(reader)?;

                Ok(container)
            })
            .await?;

        Ok(container)
    }

    /// Fetches an container by id, returning the local id
    #[instrument(skip(self))]
    pub(crate) async fn load_container_local_id(&self, id: Uuid) -> Result<Option<String>> {
        let local_id = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);
                let local_id = containers::table
                    .select(containers::local_id)
                    .filter(containers::id.eq(id))
                    .first::<Option<String>>(reader)
                    .optional()?
                    .flatten();

                Ok(local_id)
            })
            .await?;

        Ok(local_id)
    }

    /// Fetches an container by id, only if all the resources are present
    #[instrument(skip(self))]
    pub(crate) async fn find_container(&self, id: Uuid) -> Result<Option<ContainerResource>> {
        let container = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);
                let Some(container): Option<Container> = Container::find_id(&id)
                    .select(Container::as_select())
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
                debug_assert!(!has_missing_device_mappings(reader, &id).unwrap());
                debug_assert!(!has_missing_device_requests(reader, &id).unwrap());

                // Error if not found (foreign key constraint is broken).
                let image = images::table
                    .select(images::reference)
                    .filter(images::id.eq(image_id))
                    .first::<String>(reader)?;

                let mut resource =
                    ContainerResource::try_from(container).map_err(HandleError::from_app)?;
                resource.add_image(image);

                let network_ids: Vec<SqlUuid> = container_networks::table
                    .select(container_networks::network_id)
                    .filter(container_networks::container_id.eq(&id))
                    .load::<SqlUuid>(reader)?;
                resource.add_networks(network_ids);

                let envs = container_env::table
                    .select(container_env::value)
                    .filter(container_env::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_env_vars(envs);

                let binds = container_binds::table
                    .select(container_binds::value)
                    .filter(container_binds::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_binds(binds);

                let port_bindings = ContainerPortBindData::query()
                    .filter(container_port_bindings::container_id.eq(&id))
                    .order_by(container_port_bindings::idx)
                    .load(reader)?;
                resource.add_port_bindings(port_bindings);

                let extra_hosts = container_extra_hosts::table
                    .select(container_extra_hosts::value)
                    .filter(container_extra_hosts::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_extra_hosts(extra_hosts);

                let cap_add = container_add_capabilities::table
                    .select(container_add_capabilities::value)
                    .filter(container_add_capabilities::container_id.eq(&id))
                    .load::<String>(reader)?;
                let cap_drop = container_drop_capabilities::table
                    .select(container_drop_capabilities::value)
                    .filter(container_drop_capabilities::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.extend_caps(cap_add, cap_drop);

                let storage_opt = container_storage_options::table
                    .select((
                        container_storage_options::name,
                        container_storage_options::value,
                    ))
                    .filter(container_storage_options::container_id.eq(&id))
                    .load_iter(reader)?
                    .collect::<QueryResult<HashMap<String, String>>>()?;
                resource.add_storage_opt(storage_opt);

                let temp_fs = container_tmpfs::table
                    .select((container_tmpfs::path, container_tmpfs::options))
                    .filter(container_tmpfs::container_id.eq(&id))
                    .load_iter(reader)?
                    .collect::<QueryResult<HashMap<String, String>>>()?;
                resource.add_tempfs(temp_fs);

                let device_mappings = DeviceMapping::query()
                    .inner_join(container_device_mappings::table)
                    .filter(container_device_mappings::container_id.eq(&id))
                    .load::<DeviceMapping>(reader)?;
                resource.add_device_mappings(device_mappings);

                let device_requests = device_requests::table
                    .select(DeviceRequest::as_select())
                    .inner_join(container_device_requests::table)
                    .filter(container_device_requests::container_id.eq(&id))
                    .load::<DeviceRequest>(reader)?;

                for device_request in device_requests {
                    let device_request = load_stored_device_request(reader, device_request)?;

                    resource.add_device_request(device_request);
                }

                let cmds = container_cmds::table
                    .select(container_cmds::cmd)
                    .filter(container_cmds::container_id.eq(&id))
                    .order_by(container_cmds::idx)
                    .load::<String>(reader)?;
                resource.add_cmd(cmds);

                let health_check_test = container_healthcheck_test::table
                    .select(container_healthcheck_test::cmd)
                    .filter(container_healthcheck_test::container_id.eq(&id))
                    .order_by(container_healthcheck_test::idx)
                    .load::<String>(reader)?;
                resource.add_health_check_test(health_check_test);

                let entrypoint = container_entrypoints::table
                    .select(container_entrypoints::cmd)
                    .filter(container_entrypoints::container_id.eq(&id))
                    .order_by(container_entrypoints::idx)
                    .load::<String>(reader)?;
                resource.add_entrypoint(entrypoint);

                let labels = container_labels::table
                    .select((container_labels::key, container_labels::value))
                    .filter(container_labels::container_id.eq(&id))
                    .load_iter(reader)?
                    .collect::<QueryResult<HashMap<String, String>>>()?;
                resource.add_labels(labels);

                let exposed_ports = container_exposed_ports::table
                    .select(container_exposed_ports::port)
                    .filter(container_exposed_ports::container_id.eq(&id))
                    .load(reader)?;
                resource.add_exposed_ports(exposed_ports);

                let device_cgroup_rules = container_device_cgroup_rules::table
                    .select(container_device_cgroup_rules::rule)
                    .filter(container_device_cgroup_rules::container_id.eq(&id))
                    .load(reader)?;
                resource.add_device_cgroup_rules(device_cgroup_rules);

                let ulimits = ContainerUlimit::query()
                    .filter(container_ulimits::container_id.eq(&id))
                    .load(reader)?;
                resource.add_ulimits(ulimits);

                let dns = container_dns::table
                    .select(container_dns::dns)
                    .filter(container_dns::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_dns(dns);

                let dns_options = container_dns_options::table
                    .select(container_dns_options::dns_option)
                    .filter(container_dns_options::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_dns_options(dns_options);

                let dns_search = container_dns_search::table
                    .select(container_dns_search::dns_search)
                    .filter(container_dns_search::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_dns_search(dns_search);

                let groups = container_group_add::table
                    .select(container_group_add::group_add)
                    .filter(container_group_add::container_id.eq(&id))
                    .load::<String>(reader)?;
                resource.add_group_add(groups);

                let sysctl = container_sysctls::table
                    .select((container_sysctls::key, container_sysctls::value))
                    .filter(container_sysctls::container_id.eq(&id))
                    .load_iter(reader)?
                    .collect::<QueryResult<HashMap<String, String>>>()?;
                resource.add_sysctl(sysctl);

                let log_config = container_log_config::table
                    .select((container_log_config::key, container_log_config::value))
                    .filter(container_log_config::container_id.eq(&id))
                    .load_iter(reader)?
                    .collect::<QueryResult<HashMap<String, String>>>()?;
                resource.add_log_config(log_config);

                let blkio_weight_device = ContainerBlkioWeightDevice::query()
                    .filter(container_blkio_weight_device::container_id.eq(&id))
                    .load(reader)?;
                resource
                    .add_blkio_weight_device(blkio_weight_device)
                    .map_err(HandleError::from_app)?;

                let blkio_device_read_bps = ContainerBlkioDeviceReadBps::query()
                    .filter(container_blkio_device_read_bps::container_id.eq(&id))
                    .load(reader)?;
                resource.add_blkio_device_read_bps(blkio_device_read_bps);

                let blkio_device_write_bps = ContainerBlkioDeviceWriteBps::query()
                    .filter(container_blkio_device_write_bps::container_id.eq(&id))
                    .load(reader)?;
                resource.add_blkio_device_write_bps(blkio_device_write_bps);

                let blkio_device_read_iops = ContainerBlkioDeviceReadIops::query()
                    .filter(container_blkio_device_read_iops::container_id.eq(&id))
                    .load(reader)?;
                resource.add_blkio_device_read_iops(blkio_device_read_iops);

                let blkio_device_write_iops = ContainerBlkioDeviceWriteIops::query()
                    .filter(container_blkio_device_write_iops::container_id.eq(&id))
                    .load(reader)?;
                resource.add_blkio_device_write_iops(blkio_device_write_iops);

                let security_opts = container_securityopts::table
                    .select(container_securityopts::opt)
                    .filter(container_securityopts::container_id.eq(&id))
                    .load(reader)?;
                resource.add_security_opts(security_opts);

                let masked_paths = container_masked_paths::table
                    .select(container_masked_paths::path)
                    .filter(container_masked_paths::container_id.eq(&id))
                    .load(reader)?;
                resource.add_masked_paths(masked_paths);

                let read_only_paths = container_readonly_paths::table
                    .select(container_readonly_paths::path)
                    .filter(container_readonly_paths::container_id.eq(&id))
                    .load(reader)?;
                resource.add_read_only_paths(read_only_paths);

                Ok(Some(resource))
            })
            .await?;

        Ok(container)
    }

    /// Finds the unique id of the container with the given local id
    #[instrument(skip(self))]
    pub(crate) async fn find_container_by_local_id(
        &self,
        local_id: String,
    ) -> Result<Option<Uuid>> {
        let id = self
            .handle
            .for_read(|reader| {
                containers::table
                    .filter(containers::local_id.eq(local_id))
                    .select(containers::id)
                    .first::<SqlUuid>(reader)
                    .map(|id| *id)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await?;

        Ok(id)
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

fn has_missing_device_mappings(
    connection: &mut SqliteConnection,
    container_id: &SqlUuid,
) -> std::result::Result<bool, HandleError> {
    select(exists(container_missing_device_mappings::table.filter(
        container_missing_device_mappings::container_id.eq(container_id),
    )))
    .get_result::<bool>(connection)
    .map_err(HandleError::from)
}

fn has_missing_device_requests(
    connection: &mut SqliteConnection,
    container_id: &SqlUuid,
) -> std::result::Result<bool, HandleError> {
    select(exists(container_missing_device_requests::table.filter(
        container_missing_device_requests::container_id.eq(container_id),
    )))
    .get_result::<bool>(connection)
    .map_err(HandleError::from)
}

#[cfg(test)]
pub(crate) mod tests {
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;
    use crate::store::container::create::tests::stored_container_full;

    use super::*;

    pub(crate) async fn find_container(store: &StateStore, id: Uuid) -> Option<Container<'_>> {
        store
            .handle
            .for_read(move |reader| {
                Container::find_id(&SqlUuid::new(id))
                    .select(Container::as_select())
                    .first::<Container>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn update_status() {
        let tmp = TempDir::with_prefix("update_status").unwrap();
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
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let mut exp = stored_container_full(container.id.0, &image);
        exp.status = ContainerStatus::Published;

        // Otherwise the image id in the container would be missing
        store.create_image(image.clone()).await.unwrap();

        store
            .create_container(Box::new(container.clone()))
            .await
            .unwrap();

        store
            .update_container_status(exp.id.0, ContainerStatus::Published)
            .await
            .unwrap();

        let container = find_container(&store, container.id.0).await.unwrap();
        assert_eq!(container, exp);
    }

    #[tokio::test]
    async fn fetch_by_local_id() {
        let tmp = TempDir::with_prefix("fetch_by_local_id").unwrap();
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
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let exp = stored_container_full(container.id.0, &image);

        store.create_container(Box::new(container)).await.unwrap();

        let local_id = Uuid::new_v4();
        store
            .update_container_local_id(exp.id.0, Some(local_id.to_string()))
            .await
            .unwrap();

        let res = store
            .find_container_by_local_id(local_id.to_string())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(res, exp.id.0);
    }

    #[tokio::test]
    async fn should_load_container_in_state() {
        let tmp = TempDir::with_prefix("load_containers_in_state").unwrap();
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
        let container = create_container_req(
            deployment_id,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let exp = stored_container_full(container.id.0, &image);

        store.create_container(Box::new(container)).await.unwrap();

        let containers = store
            .load_containers_in_state(vec![ContainerStatus::Received])
            .await
            .unwrap();

        assert_eq!(containers, vec![(SqlUuid::new(exp.id.0), None)]);
    }
}
