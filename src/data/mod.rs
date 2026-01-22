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

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::store::SqliteStore;
use astarte_device_sdk::store::sqlite::SqliteError;
use astarte_device_sdk::types::AstarteData;
use futures::TryFutureExt;
use std::path::Path;
use tracing::{debug, error, info};

use crate::Client;

pub mod astarte_device_sdk_lib;
#[cfg(feature = "message-hub")]
pub mod astarte_message_hub_node;

/// Connect to the store.
pub async fn connect_store<P>(store_dir: P) -> Result<SqliteStore, SqliteError>
where
    P: AsRef<Path>,
{
    let db_path = store_dir.as_ref().join("database.db");

    debug!("connecting to store {}", db_path.display());
    let store = SqliteStore::connect_db(&db_path).await?;

    info!("connected to store {}", db_path.display());
    Ok(store)
}

/// Publishes the value to Astarte and logs if an error happens.
///
/// This is used to send telemetry data without returning an error.
pub(crate) async fn send_object_with_timestamp<C, T>(
    client: &mut C,
    interface: &str,
    path: &str,
    data: T,
    timestamp: DateTime<Utc>,
) where
    C: Client,
    T: TryInto<AstarteObject, Error = astarte_device_sdk::Error>,
{
    let res = futures::future::ready(data.try_into())
        .and_then(|data| client.send_object_with_timestamp(interface, path, data, timestamp))
        .await;

    if let Err(err) = res {
        error!(
            error = format!("{:#}", eyre::Report::new(err)),
            interface, path, "failed to publish",
        )
    }
}

/// Sets the property and logs if an error happens.
///
/// This is used to send telemetry data without returning an error.
pub(crate) async fn set_property<C>(
    client: &mut C,
    interface: &str,
    path: &str,
    data: impl Into<AstarteData>,
) where
    C: Client,
{
    if let Err(err) = client.set_property(interface, path, data.into()).await {
        error!(
            error = format!("{:#}", eyre::Report::new(err)),
            interface, path, "failed to set property",
        )
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use tempdir::TempDir;

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
