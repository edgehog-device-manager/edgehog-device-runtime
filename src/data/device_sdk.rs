// This file is part of Edgehog.
//
// Copyright 2022, 2026 SECO Mind Srl
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

use astarte_device_sdk::DeviceClient;
use astarte_device_sdk::builder::DeviceBuilder;
use astarte_device_sdk::pairing::api::PairingApi;
use astarte_device_sdk::prelude::*;
use astarte_device_sdk::store::SqliteStore;
use astarte_device_sdk::transport::mqtt::{Credential, Mqtt, MqttArgs, MqttConfig};
use edgehog_config::sdk::DeviceSdkConfig;
use eyre::{Context, OptionExt};
use serde::Deserialize;
use tokio::task::JoinSet;
use url::Url;

use crate::DeviceManagerOptions;

use super::interfaces::add_interfaces;

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceSdkOptions {
    pub realm: String,
    pub device_id: String,
    pub pairing_url: Url,
    pub credentials: Credential,
    pub ignore_ssl: bool,
}

impl DeviceSdkOptions {
    pub async fn connect(
        &self,
        tasks: &mut JoinSet<eyre::Result<()>>,
        store: SqliteStore,
        options: &DeviceManagerOptions,
    ) -> eyre::Result<DeviceClient<Mqtt<SqliteStore, PairingApi>>> {
        let mut mqtt_cfg = MqttConfig::new(MqttArgs {
            realm: self.realm.clone(),
            device_id: self.device_id.clone(),
            credential: self.credentials.clone(),
            pairing_url: self.pairing_url.clone(),
        });

        if self.ignore_ssl {
            mqtt_cfg = mqtt_cfg.ignore_ssl_errors();
        }

        let (client, connection) = add_interfaces(DeviceBuilder::new(), options)
            .wrap_err("couldn't add interfaces")?
            .writable_dir(&options.store_directory)
            .store(store)
            .connection(mqtt_cfg)
            .build()
            .await
            .wrap_err("couldn't build device connection")?;

        tasks.spawn(async move { connection.handle_events().await.map_err(Into::into) });

        Ok(client)
    }
}

impl TryFrom<DeviceSdkConfig> for DeviceSdkOptions {
    type Error = eyre::Report;

    fn try_from(value: DeviceSdkConfig) -> Result<Self, Self::Error> {
        let DeviceSdkConfig {
            realm,
            device_id,
            credentials_secret,
            pairing_token,
            pairing_url,
            ignore_ssl,
        } = value;

        let realm = realm.ok_or_eyre("missing Astarte realm")?;
        let device_id = device_id.ok_or_eyre("missing Astarte realm")?;
        let pairing_url = pairing_url.ok_or_eyre("missing Pairing URL")?;
        let credentials = credentials_secret
            .map(Credential::secret)
            .or_else(|| pairing_token.map(Credential::paring_token))
            .ok_or_eyre("missing both credential secret and pairing token")?;
        let ignore_ssl = ignore_ssl.unwrap_or(false);

        Ok(DeviceSdkOptions {
            realm,
            device_id,
            credentials,
            pairing_url,
            ignore_ssl,
        })
    }
}

#[cfg(all(feature = "zbus", target_os = "linux"))]
pub async fn hardware_id_from_dbus() -> eyre::Result<Option<String>> {
    let connection = zbus::Connection::system().await?;
    let proxy = crate::device::DeviceProxy::new(&connection).await?;
    let hardware_id: String = proxy.get_hardware_id("").await?;

    if hardware_id.is_empty() {
        return Ok(None);
    }

    Ok(Some(hardware_id))
}
