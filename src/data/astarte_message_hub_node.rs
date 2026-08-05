// This file is part of Edgehog.
//
// Copyright 2023, 2026 SECO Mind Srl
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

//! Contains the implementation for the Astarte message hub node.

use std::path::Path;

use astarte_device_sdk::DeviceClient;
use astarte_device_sdk::builder::DeviceBuilder;
use astarte_device_sdk::prelude::*;
use astarte_device_sdk::store::SqliteStore;
use astarte_device_sdk::transport::grpc::{Grpc, GrpcConfig};
use eyre::Context;
use serde::Deserialize;
use tokio::task::JoinSet;
use url::Url;
use uuid::{Uuid, uuid};

use super::interfaces::add_interfaces;

/// Device runtime node identifier.
const DEVICE_RUNTIME_NODE_UUID: Uuid = uuid!("d72a6187-7cf1-44cc-87e8-e991936166db");

/// Struct containing the configuration options for the Astarte message hub.
#[derive(Debug, Deserialize, Clone)]
pub struct AstarteMessageHubOptions {
    /// The Endpoint of the Astarte Message Hub
    pub endpoint: Url,
}

impl AstarteMessageHubOptions {
    pub async fn connect<P>(
        &self,
        tasks: &mut JoinSet<eyre::Result<()>>,
        store: SqliteStore,
        store_dir: P,
    ) -> eyre::Result<DeviceClient<Grpc<SqliteStore>>>
    where
        P: AsRef<Path>,
    {
        let grpc_cfg = GrpcConfig::from_url(DEVICE_RUNTIME_NODE_UUID, self.endpoint.to_string())
            .wrap_err("invalid message-hub endpoint")?;

        let (device, connection) = add_interfaces(DeviceBuilder::new())
            .wrap_err("couldn't add interfaces")?
            .writable_dir(store_dir)
            .store(store)
            .connection(grpc_cfg)
            .build()
            .await
            .wrap_err("couldn't connect to Astarte")?;

        tasks.spawn(async move { connection.handle_events().await.map_err(Into::into) });

        Ok(device)
    }
}
