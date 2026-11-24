// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

use diesel::query_dsl::methods::SelectDsl;
use diesel::{ExpressionMethods, RunQueryDsl, delete, insert_or_ignore_into, update};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::network_options::{
    NetworkOptions, NetworkOptionsAlias, NetworkOptionsDnsName, NetworkOptionsDriverOpt,
    NetworkOptionsLink, NetworkOptionsLinkLocalIp,
};
use edgehog_store::models::containers::network_options::{
    NetworkOptionsStatus, NetworkOptionsVecFields,
};
use edgehog_store::schema::containers::{
    network_options, network_options_aliases, network_options_dns_names,
    network_options_driver_opts, network_options_link_local_ips, network_options_links,
};
use tracing::instrument;
use uuid::Uuid;

use crate::requests::network_options::CreateNetworkOptions;

use super::{Result, StateStore};

impl StateStore {
    /// Stores the network options received from the CreateRequest
    #[instrument(skip_all, fields(%create_network_options.id))]
    pub(crate) async fn create_network_options(
        &self,
        create_network_options: Box<CreateNetworkOptions>,
    ) -> Result<()> {
        let net_options: NetworkOptions = NetworkOptions::from(*create_network_options.clone());
        let net_options_vec_fields = NetworkOptionsVecFields::from(*create_network_options);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(network_options::table)
                    .values(&net_options)
                    .execute(writer)?;
                if let Some(network_options_link_local_ips) =
                    net_options_vec_fields.network_options_link_local_ips
                {
                    for link_local_ip in network_options_link_local_ips {
                        insert_or_ignore_into(network_options_link_local_ips::table)
                            .values(&link_local_ip)
                            .execute(writer)?;
                    }
                }
                if let Some(network_options_links) = net_options_vec_fields.network_options_links {
                    for link in network_options_links {
                        insert_or_ignore_into(network_options_links::table)
                            .values(&link)
                            .execute(writer)?;
                    }
                }
                if let Some(network_options_driver_opts) =
                    net_options_vec_fields.network_options_driver_opts
                {
                    for driver_opt in network_options_driver_opts {
                        insert_or_ignore_into(network_options_driver_opts::table)
                            .values(&driver_opt)
                            .execute(writer)?;
                    }
                }
                if let Some(network_options_dns_names) =
                    net_options_vec_fields.network_options_dns_names
                {
                    for dns_name in network_options_dns_names {
                        insert_or_ignore_into(network_options_dns_names::table)
                            .values(&dns_name)
                            .execute(writer)?;
                    }
                }
                if let Some(network_options_aliases) =
                    net_options_vec_fields.network_options_aliases
                {
                    for alias in network_options_aliases {
                        insert_or_ignore_into(network_options_aliases::table)
                            .values(&alias)
                            .execute(writer)?;
                    }
                }

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the state of a network_options
    #[instrument(skip(self))]
    pub(crate) async fn update_network_options_status(
        &self,
        network_options_id: Uuid,
        status: NetworkOptionsStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(NetworkOptions::find_id(&SqlUuid::new(network_options_id)))
                    .set(network_options::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a [`NetworkOptions`]
    #[instrument(skip(self))]
    pub(crate) async fn delete_network_options(&self, network_options_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(NetworkOptions::find_id(&SqlUuid::new(network_options_id)))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_network_options_to_publish(&self) -> Result<Vec<SqlUuid>> {
        let network_options = self
            .handle
            .for_read(move |reader| {
                let network_options = network_options::table
                    .select(network_options::id)
                    .load::<SqlUuid>(reader)?;

                Ok(network_options)
            })
            .await?;

        Ok(network_options)
    }
}

impl From<CreateNetworkOptions> for NetworkOptions {
    fn from(
        CreateNetworkOptions {
            id,
            deployment_id: _,
            container_id,
            network_id,
            ipv4_address,
            ipv6_address,
            mac_address,
            gw_priority,
            gateway,
            ip_address,
            ip_prefix_len,
            ipv6_gateway,
            global_ipv6_address,
            global_ipv6_prefix_len,
            ..
        }: CreateNetworkOptions,
    ) -> Self {
        Self {
            id: SqlUuid::new(id),
            container_id: SqlUuid::new(container_id),
            network_id: SqlUuid::new(network_id),
            status: NetworkOptionsStatus::default(),
            mac_address,
            ipv4_address,
            ipv6_address,
            gw_priority,
            gateway,
            ip_address,
            ip_prefix_len,
            ipv6_gateway,
            global_ipv6_address,
            global_ipv6_prefix_len,
        }
    }
}

impl From<CreateNetworkOptions> for NetworkOptionsVecFields {
    fn from(value: CreateNetworkOptions) -> Self {
        Self {
            network_options_link_local_ips: {
                if let Some(link_local_ips) = value.link_local_ips {
                    if link_local_ips.is_empty() {
                        None
                    } else {
                        Some(
                            link_local_ips
                                .into_iter()
                                .map(|ip| NetworkOptionsLinkLocalIp {
                                    network_options_id: SqlUuid::from(Uuid::from(value.id)),
                                    ip,
                                })
                                .collect(),
                        )
                    }
                } else {
                    None
                }
            },

            network_options_links: {
                if let Some(links) = value.links {
                    if links.is_empty() {
                        None
                    } else {
                        Some(
                            links
                                .into_iter()
                                .map(|link| NetworkOptionsLink {
                                    network_options_id: SqlUuid::from(Uuid::from(value.id)),
                                    link,
                                })
                                .collect(),
                        )
                    }
                } else {
                    None
                }
            },
            network_options_driver_opts: {
                if let Some(keys) = value.driver_opt_keys {
                    if let Some(values) = value.driver_opt_values {
                        if keys.is_empty() || values.is_empty() {
                            None
                        } else {
                            Some(
                                keys.into_iter()
                                    .zip(values)
                                    .map(|(key, val)| NetworkOptionsDriverOpt {
                                        network_options_id: SqlUuid::from(Uuid::from(value.id)),
                                        option_key: key,
                                        option_value: val,
                                    })
                                    .collect(),
                            )
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            },

            network_options_dns_names: {
                if let Some(dns_names) = value.dns_names {
                    if dns_names.is_empty() {
                        None
                    } else {
                        Some(
                            dns_names
                                .into_iter()
                                .map(|dns_name| NetworkOptionsDnsName {
                                    network_options_id: SqlUuid::from(Uuid::from(value.id)),
                                    dns_name,
                                })
                                .collect(),
                        )
                    }
                } else {
                    None
                }
            },

            network_options_aliases: {
                if let Some(aliases) = value.aliases {
                    if aliases.is_empty() {
                        None
                    } else {
                        Some(
                            aliases
                                .into_iter()
                                .map(|alias| NetworkOptionsAlias {
                                    network_options_id: SqlUuid::from(Uuid::from(value.id)),
                                    alias,
                                })
                                .collect(),
                        )
                    }
                } else {
                    None
                }
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {

    use crate::requests::container::tests::create_container_req;
    use crate::requests::deployment::CreateDeployment;
    use crate::requests::{ReqUuid, VecReqUuid};

    use super::*;

    use diesel::OptionalExtension;
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::env_file::tests::create_env_file_req;
    use crate::requests::file_bind::tests::create_file_bind_req;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::network_options::tests::create_network_options_req;
    use crate::requests::volume::tests::create_volume_req;

    async fn find_network_options(store: &StateStore, id: Uuid) -> Option<NetworkOptions> {
        store
            .handle
            .for_read(move |reader| {
                NetworkOptions::find_id(&SqlUuid::new(id))
                    .first::<NetworkOptions>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_network_options").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let network = create_network_req(deployment_id);
        let volume = create_volume_req(deployment_id);
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
        let network_options = create_network_options_req(
            deployment_id,
            Uuid::from(container.id),
            Uuid::from(network.id),
        );
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };

        let network_id = network.id;
        let container_id = container.id;
        let network_options_id = network_options.id;
        store.create_image(image).await.unwrap();
        store.create_volume(volume).await.unwrap();
        store.create_network(network).await.unwrap();
        store.create_device_mapping(device_mapping).await.unwrap();
        store.create_device_request(device_request).await.unwrap();
        store.create_file_bind(file_bind).await.unwrap();
        store.create_env_file(env_file).await.unwrap();
        store.create_deployment(deployment).await.unwrap();
        store.create_container(Box::new(container)).await.unwrap();
        store
            .create_network_options(Box::new(network_options))
            .await
            .unwrap();

        let res = find_network_options(&store, Uuid::from(network_options_id))
            .await
            .unwrap();

        let exp = NetworkOptions {
            id: SqlUuid::new(Uuid::from(network_options_id)),
            container_id: SqlUuid::new(Uuid::from(container_id)),
            network_id: SqlUuid::new(Uuid::from(network_id)),
            status: NetworkOptionsStatus::default(),
            ipv4_address: None,
            ipv6_address: None,
            mac_address: None,
            gw_priority: None,
            gateway: None,
            ip_address: None,
            ip_prefix_len: None,
            ipv6_gateway: None,
            global_ipv6_address: None,
            global_ipv6_prefix_len: None,
        };

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_network_options").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let network_options_id = Uuid::new_v4();

        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let network = create_network_req(deployment_id);
        let volume = create_volume_req(deployment_id);
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
        let deployment = CreateDeployment {
            id: ReqUuid(deployment_id),
            containers: VecReqUuid(vec![container.id]),
        };
        let network_options = CreateNetworkOptions {
            id: ReqUuid(network_options_id),
            deployment_id: ReqUuid(deployment_id),
            container_id: container.id,
            network_id: network.id,
            ipv4_address: None,
            ipv6_address: None,
            link_local_ips: None,
            links: None,
            mac_address: None,
            aliases: None,
            driver_opt_keys: None,
            driver_opt_values: None,
            gw_priority: None,
            gateway: None,
            ip_address: None,
            ip_prefix_len: None,
            ipv6_gateway: None,
            global_ipv6_address: None,
            global_ipv6_prefix_len: None,
            dns_names: None,
        };

        let network_id = network.id;
        let container_id = container.id;
        store.create_image(image).await.unwrap();
        store.create_volume(volume).await.unwrap();
        store.create_network(network).await.unwrap();
        store.create_device_mapping(device_mapping).await.unwrap();
        store.create_device_request(device_request).await.unwrap();
        store.create_file_bind(file_bind).await.unwrap();
        store.create_env_file(env_file).await.unwrap();
        store.create_deployment(deployment).await.unwrap();
        store.create_container(Box::new(container)).await.unwrap();
        store
            .create_network_options(Box::new(network_options))
            .await
            .unwrap();

        store
            .update_network_options_status(network_options_id, NetworkOptionsStatus::Received)
            .await
            .unwrap();

        let res = find_network_options(&store, network_options_id)
            .await
            .unwrap();

        let exp = NetworkOptions {
            id: SqlUuid::new(network_options_id),
            container_id: SqlUuid::new(Uuid::from(container_id)),
            network_id: SqlUuid::new(Uuid::from(network_id)),
            status: NetworkOptionsStatus::Received,
            ipv4_address: None,
            ipv6_address: None,
            mac_address: None,
            gw_priority: None,
            gateway: None,
            ip_address: None,
            ip_prefix_len: None,
            ipv6_gateway: None,
            global_ipv6_address: None,
            global_ipv6_prefix_len: None,
        };

        assert_eq!(res, exp);
    }
}
