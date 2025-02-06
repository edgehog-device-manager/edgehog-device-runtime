// This file is part of Astarte.
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

use diesel::query_dsl::methods::{FilterDsl, SelectDsl};
use diesel::{
    delete, insert_or_ignore_into, update, ExpressionMethods, OptionalExtension, RunQueryDsl,
};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::containers::container::ContainerMissingNetwork;
use edgehog_store::models::containers::network::{Network, NetworkDriverOpts, NetworkStatus};
use edgehog_store::models::QueryModel;
use edgehog_store::schema::containers::{container_networks, network_driver_opts, networks};
use itertools::Itertools;
use tracing::instrument;
use uuid::Uuid;

use crate::resource::network::NetworkResource;
use crate::{docker::network::Network as ContainerNetwork, requests::network::CreateNetwork};

use super::{split_key_value, Result, StateStore, StoreError};

impl StateStore {
    /// Stores the network received from the CreateRequest
    #[instrument(skip_all, fields(%create_network.id))]
    pub(crate) async fn create_network(&self, create_network: CreateNetwork) -> Result<()> {
        let opts = Vec::<NetworkDriverOpts>::try_from(&create_network)?;
        let network = Network::from(create_network);

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(networks::table)
                    .values(&network)
                    .execute(writer)?;

                insert_or_ignore_into(network_driver_opts::table)
                    .values(opts)
                    .execute(writer)?;

                insert_or_ignore_into(container_networks::table)
                    .values(ContainerMissingNetwork::find_by_network(&network.id))
                    .execute(writer)?;

                delete(ContainerMissingNetwork::find_by_network(&network.id)).execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the state of a network
    #[instrument(skip(self))]
    pub(crate) async fn update_network_status(
        &self,
        network_id: Uuid,
        status: NetworkStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Network::find_id(&SqlUuid::new(network_id)))
                    .set(networks::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the local id of a [`Network`]
    #[instrument(skip(self))]
    pub(crate) async fn update_network_local_id(
        &self,
        network_id: Uuid,
        local_id: Option<String>,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(Network::find_id(&SqlUuid::new(network_id)))
                    .set(networks::local_id.eq(local_id))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a [`Network`]
    #[instrument(skip(self))]
    pub(crate) async fn delete_network(&self, network_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated =
                    delete(Network::find_id(&SqlUuid::new(network_id))).execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_networks_to_publish(&mut self) -> Result<Vec<SqlUuid>> {
        let networks = self
            .handle
            .for_read(move |reader| {
                let networks = networks::table
                    .select(networks::id)
                    .filter(networks::status.eq(NetworkStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(networks)
            })
            .await?;

        Ok(networks)
    }

    #[instrument(skip(self))]
    pub(crate) async fn find_network(
        &mut self,
        network_id: Uuid,
    ) -> Result<Option<NetworkResource>> {
        let network = self
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(network_id);
                let Some(network) = Network::find_id(&id).first::<Network>(reader).optional()?
                else {
                    return Ok(None);
                };

                let driver_opts = network_driver_opts::table
                    .filter(network_driver_opts::network_id.eq(id))
                    .load::<NetworkDriverOpts>(reader)?
                    .into_iter()
                    .map(|opt| (opt.name, opt.value))
                    .collect();

                Ok(Some(NetworkResource::new(ContainerNetwork {
                    id: network.local_id,
                    name: network.id.to_string(),
                    driver: network.driver,
                    internal: network.internal,
                    enable_ipv6: network.enable_ipv6,
                    driver_opts,
                })))
            })
            .await?;

        Ok(network)
    }
}

impl From<CreateNetwork> for Network {
    fn from(
        CreateNetwork {
            id,
            driver,
            internal,
            enable_ipv6,
            options: _,
        }: CreateNetwork,
    ) -> Self {
        Self {
            id: SqlUuid::new(id),
            local_id: None,
            status: NetworkStatus::default(),
            driver: driver.to_string(),
            internal,
            enable_ipv6,
        }
    }
}

// Takes the options
impl TryFrom<&CreateNetwork> for Vec<NetworkDriverOpts> {
    type Error = StoreError;

    fn try_from(value: &CreateNetwork) -> std::result::Result<Self, Self::Error> {
        let network_id = SqlUuid::new(value.id);

        value
            .options
            .iter()
            .map(|s| {
                split_key_value(s)
                    .map(|(name, value)| NetworkDriverOpts {
                        network_id,
                        name: name.to_string(),
                        value: value.unwrap_or_default().to_string(),
                    })
                    .ok_or(StoreError::ParseKeyValue {
                        ctx: "network driver options",
                        value: s.to_string(),
                    })
            })
            .try_collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::requests::ReqUuid;

    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    async fn find_network(store: &mut StateStore, id: Uuid) -> Option<Network> {
        store
            .handle
            .for_read(move |reader| {
                Network::find_id(&SqlUuid::new(id))
                    .first::<Network>(reader)
                    .optional()
                    .map_err(HandleError::Query)
            })
            .await
            .unwrap()
    }

    impl StateStore {
        pub(crate) async fn network_opts(
            &mut self,
            network_id: Uuid,
        ) -> Result<Vec<NetworkDriverOpts>> {
            let network = self
                .handle
                .for_read(move |reader| {
                    let network: Vec<NetworkDriverOpts> = network_driver_opts::table
                        .filter(network_driver_opts::network_id.eq(SqlUuid::new(network_id)))
                        .load(reader)?;

                    Ok(network)
                })
                .await?;

            Ok(network)
        }
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_network").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let network_id = Uuid::new_v4();
        let network = CreateNetwork {
            id: ReqUuid(network_id),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let res = find_network(&mut store, network_id).await.unwrap();

        let exp = Network {
            id: SqlUuid::new(network_id),
            local_id: None,
            status: NetworkStatus::Received,
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
        };

        assert_eq!(res, exp);

        let network_opts = store.network_opts(network_id).await.unwrap();

        assert_eq!(
            network_opts,
            vec![NetworkDriverOpts {
                network_id: SqlUuid::new(network_id),
                name: "isolate".to_string(),
                value: "true".to_string()
            }]
        );
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_network").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let mut store = StateStore::new(handle);

        let network_id = Uuid::new_v4();
        let network = CreateNetwork {
            id: ReqUuid(network_id),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        store
            .update_network_status(network_id, NetworkStatus::Published)
            .await
            .unwrap();

        let res = find_network(&mut store, network_id).await.unwrap();

        let exp = Network {
            id: SqlUuid::new(network_id),
            local_id: None,
            status: NetworkStatus::Published,
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
        };

        assert_eq!(res, exp);
    }
}
