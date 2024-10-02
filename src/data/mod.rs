/*
 * This file is part of Edgehog.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use astarte_device_sdk::client::RecvError;
use astarte_device_sdk::properties::PropAccess;
use astarte_device_sdk::store::sqlite::SqliteError;
use astarte_device_sdk::store::{SqliteStore, StoredProp};
use astarte_device_sdk::types::AstarteType;
use astarte_device_sdk::{
    error::Error as AstarteError, AstarteAggregate, Client, DeviceClient, DeviceEvent,
};
use async_trait::async_trait;
use std::path::Path;
use tracing::{debug, error, info};

pub mod astarte_device_sdk_lib;
#[cfg(feature = "message-hub")]
pub mod astarte_message_hub_node;

#[async_trait]
pub trait Publisher {
    async fn send_object<T>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), AstarteError>
    where
        T: AstarteAggregate + Send + 'static;
    async fn send(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: AstarteType,
    ) -> Result<(), AstarteError>;
    async fn interface_props(&self, interface: &str) -> Result<Vec<StoredProp>, AstarteError>;
    async fn unset(&self, interface_name: &str, interface_path: &str) -> Result<(), AstarteError>;
}

#[async_trait]
pub trait Subscriber {
    async fn recv(&self) -> Result<DeviceEvent, RecvError>;
}

#[async_trait]
impl Publisher for DeviceClient<SqliteStore> {
    async fn send_object<T: 'static>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), AstarteError>
    where
        T: AstarteAggregate + Send,
    {
        Client::send_object(self, interface_name, interface_path, data).await
    }

    async fn send(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: AstarteType,
    ) -> Result<(), AstarteError> {
        Client::send(self, interface_name, interface_path, data).await
    }

    async fn interface_props(&self, interface: &str) -> Result<Vec<StoredProp>, AstarteError> {
        PropAccess::interface_props(self, interface).await
    }

    async fn unset(&self, interface_name: &str, interface_path: &str) -> Result<(), AstarteError> {
        Client::unset(self, interface_name, interface_path).await
    }
}

#[async_trait]
impl Subscriber for DeviceClient<SqliteStore> {
    async fn recv(&self) -> Result<DeviceEvent, RecvError> {
        Client::recv(self).await
    }
}

/// Store errors
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum StoreError {
    /// couldn't connect to Sqlite store
    Connect(#[source] SqliteError),
}

/// Connect to the store.
pub async fn connect_store<P>(store_dir: P) -> Result<SqliteStore, StoreError>
where
    P: AsRef<Path>,
{
    let db_path = store_dir.as_ref().join("database.db");

    debug!("connecting to store {}", db_path.display());
    let store = SqliteStore::connect_db(&db_path)
        .await
        .map_err(StoreError::Connect)?;

    info!("connected to store {}", db_path.display());
    Ok(store)
}

/// Publishes the value to Astarte and logs if an error happens.
///
/// This is used to send telemetry data without returning an error.
pub(crate) async fn publish<T>(
    client: &T,
    interface: &str,
    path: &str,
    data: impl Into<AstarteType>,
) where
    T: Publisher,
{
    if let Err(err) = client.send(interface, path, data.into()).await {
        error!(
            "failed to send {}{path}: {}",
            interface,
            stable_eyre::Report::new(err)
        )
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use mockall::mock;
    use tempdir::TempDir;

    mock! {
        pub PubSub {}

        #[async_trait]
        impl Publisher for PubSub {
            async fn send_object<T>(
                &self,
                interface_name: &str,
                interface_path: &str,
                data: T,
            ) -> Result<(), AstarteError>
            where
                T: AstarteAggregate + Send + 'static;
            async fn send(
                &self,
                interface_name: &str,
                interface_path: &str,
                data: AstarteType,
            ) -> Result<(), AstarteError>;
            async fn interface_props(&self, interface: &str) -> Result<Vec<StoredProp>, AstarteError>;
            async fn unset(
                &self,
                interface_name: &str,
                interface_path: &str
            ) -> Result<(), AstarteError>;
        }

        #[async_trait]
        impl Subscriber for PubSub {
             async fn recv(&self) -> Result<DeviceEvent, RecvError>;
        }

        impl Clone for PubSub {
            fn clone(&self) -> Self;
        }
    }

    /// Create tmp store and store dir.
    pub async fn create_tmp_store() -> (SqliteStore, TempDir) {
        let tmp_dir = TempDir::new("edgehog-tmp-store").expect("failed to create tmp store dir");
        let store = connect_store(tmp_dir.path())
            .await
            .expect("failed to connect store");

        (store, tmp_dir)
    }

    #[tokio::test]
    async fn test_connect_store() {
        create_tmp_store().await;
    }
}
