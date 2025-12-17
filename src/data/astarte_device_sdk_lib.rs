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

use std::path::Path;

use astarte_device_sdk::builder::{BuilderError, DeviceBuilder};
use astarte_device_sdk::error::Error as AstarteError;
use astarte_device_sdk::introspection::AddInterfaceError;
use astarte_device_sdk::prelude::*;
use astarte_device_sdk::store::SqliteStore;
use astarte_device_sdk::transport::mqtt::{Credential, MqttConfig, PairingError};
use astarte_device_sdk::DeviceClient;
use serde::Deserialize;
use tokio::task::JoinSet;
use url::Url;

use crate::repository::file_state_repository::{FileStateError, FileStateRepository};
use crate::repository::StateRepository;

/// Error returned by the [`astarte_device_sdk`].
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum DeviceSdkError {
    /// missing device ID
    MissingDeviceId,
    /// couldn't get the hardware id from DBus
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    Zbus(#[from] zbus::Error),
    /// couldn't pair device to Astarte
    Pairing(#[from] PairingError),
    /// couldn't write credential secret
    WriteSecret(#[source] FileStateError),
    /// couldn't read credential secret
    ReadSecret(#[source] FileStateError),
    /// couldn't get credential secret or pairing token
    MissingCredentialSecret,
    /// couldn't add interfaces directory
    Interfaces(#[from] AddInterfaceError),
    /// couldn't build Astarte device
    Builder(#[from] BuilderError),
    /// couldn't connect to Astarte
    Connect(#[source] AstarteError),
}

#[derive(Debug, Deserialize, Clone)]
pub struct AstarteDeviceSdkConfigOptions {
    pub realm: String,
    pub device_id: Option<String>,
    pub credentials_secret: Option<String>,
    pub pairing_url: Url,
    pub pairing_token: Option<String>,
    #[serde(default)]
    pub ignore_ssl: bool,
}

impl AstarteDeviceSdkConfigOptions {
    async fn device_id_or_from_dbus(&self) -> Result<String, DeviceSdkError> {
        if let Some(id) = self.device_id.as_ref().filter(|id| !id.is_empty()) {
            return Ok(id.clone());
        }

        cfg_if::cfg_if! {
            if #[cfg(all(feature = "zbus", target_os = "linux"))] {
                hardware_id_from_dbus()
                    .await?
                    .ok_or(DeviceSdkError::MissingDeviceId)
            } else {
                Err(DeviceSdkError::MissingDeviceId)
            }
        }
    }

    async fn credentials_secret(
        &self,
        device_id: &str,
        store_directory: impl AsRef<Path>,
    ) -> Result<Credential, DeviceSdkError> {
        let cred = self.credentials_secret.as_ref().filter(|id| !id.is_empty());

        if let Some(secret) = cred {
            return Ok(Credential::secret(secret));
        }

        let registry = FileStateRepository::new(
            store_directory.as_ref(),
            format!("credentials_{}.json", device_id),
        );

        if StateRepository::<String>::exists(&registry).await {
            return registry
                .read()
                .await
                .map(Credential::secret)
                .map_err(DeviceSdkError::ReadSecret);
        }

        if let Some(token) = &self.pairing_token {
            return Ok(Credential::paring_token(token));
        }

        Err(DeviceSdkError::MissingCredentialSecret)
    }

    pub async fn connect<P>(
        &self,
        tasks: &mut JoinSet<stable_eyre::Result<()>>,
        store: SqliteStore,
        store_dir: P,
        interface_dir: P,
    ) -> Result<DeviceClient<SqliteStore>, DeviceSdkError>
    where
        P: AsRef<Path>,
    {
        let device_id = self.device_id_or_from_dbus().await?;

        let credentials_secret = self.credentials_secret(&device_id, &store_dir).await?;

        let mut mqtt_cfg = MqttConfig::new(
            &self.realm,
            &device_id,
            credentials_secret,
            self.pairing_url.clone(),
        );

        if self.ignore_ssl {
            mqtt_cfg.ignore_ssl_errors();
        }

        let (client, connection) = DeviceBuilder::new()
            .store(store)
            .interface_directory(interface_dir)?
            .writable_dir(store_dir)?
            .connect(mqtt_cfg)
            .await
            .map_err(DeviceSdkError::Connect)?
            .build()
            .await;

        tasks.spawn(async move { connection.handle_events().await.map_err(Into::into) });

        Ok(client)
    }
}

#[cfg(all(feature = "zbus", target_os = "linux"))]
pub async fn hardware_id_from_dbus() -> Result<Option<String>, DeviceSdkError> {
    let connection = zbus::Connection::system().await?;
    let proxy = crate::device::DeviceProxy::new(&connection).await?;
    let hardware_id: String = proxy.get_hardware_id("").await?;

    if hardware_id.is_empty() {
        return Ok(None);
    }

    Ok(Some(hardware_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempdir::TempDir;

    #[tokio::test]
    async fn device_id_test() {
        let opts = AstarteDeviceSdkConfigOptions {
            realm: "foo".to_string(),
            device_id: Some("target".to_string()),
            credentials_secret: None,
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: None,
            ignore_ssl: false,
        };

        let id = opts.device_id_or_from_dbus().await.unwrap();
        assert_eq!(id, "target");
    }

    #[tokio::test]
    async fn credentials_secret_test() {
        let _dir = TempDir::new("sdk_cred").unwrap();
        let path = _dir.path().to_owned();

        let options = AstarteDeviceSdkConfigOptions {
            realm: "".to_string(),
            device_id: None,
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: None,
            ignore_ssl: false,
        };

        let secret = options.credentials_secret("device_id", path).await.unwrap();

        assert_eq!(secret, Credential::secret("credentials_secret"));
    }

    #[tokio::test]
    async fn credentials_pairing_token() {
        let _dir = TempDir::new("sdk_cred").unwrap();
        let path = _dir.path().to_owned();

        let options = AstarteDeviceSdkConfigOptions {
            realm: "".to_string(),
            device_id: None,
            credentials_secret: None,
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: Some("pairing_token".to_string()),
            ignore_ssl: false,
        };

        let secret = options.credentials_secret("device_id", path).await.unwrap();

        assert_eq!(secret, Credential::paring_token("pairing_token"));
    }

    #[tokio::test]
    async fn not_enough_arguments_credentials_secret_test() {
        let _dir = TempDir::new("sdk_cred").unwrap();
        let path = _dir.path().to_owned();

        let options = AstarteDeviceSdkConfigOptions {
            realm: "".to_string(),
            device_id: None,
            credentials_secret: None,
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: None,
            ignore_ssl: false,
        };

        let res = options.credentials_secret("device_id", &path).await;

        assert!(res.is_err());
    }

    #[tokio::test]
    async fn get_credentials_secret_persistence_fail() {
        let _dir = TempDir::new("sdk_cred").unwrap();
        let path = _dir.path().to_owned();

        let device_id = "device_id";

        let path = path.join(format!("credentials_{}.json", device_id));

        tokio::fs::write(&path, b"\0").await.unwrap();

        let options = AstarteDeviceSdkConfigOptions {
            realm: "".to_string(),
            device_id: Some(device_id.to_owned()),
            credentials_secret: None,
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: None,
            ignore_ssl: true,
        };

        let res = options.credentials_secret(device_id, path).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn get_credentials_secret_persistence_success() {
        let _dir = TempDir::new("sdk_cred").unwrap();
        let path = _dir.path().to_owned();

        let device_id = "device_id";

        let full_path = path.join(format!("credentials_{}.json", device_id));

        let exp = "credential_secret";

        tokio::fs::write(&full_path, format!("\"{exp}\""))
            .await
            .unwrap();

        let options = AstarteDeviceSdkConfigOptions {
            realm: "".to_string(),
            device_id: Some(device_id.to_owned()),
            credentials_secret: None,
            pairing_url: Url::parse("http://[::]").unwrap(),
            pairing_token: None,
            ignore_ssl: false,
        };

        let secret = options.credentials_secret(device_id, path).await.unwrap();

        assert_eq!(secret, Credential::secret(exp));
    }
}
