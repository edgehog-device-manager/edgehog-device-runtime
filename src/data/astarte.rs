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

use astarte_sdk::builder::AstarteOptions;
use astarte_sdk::types::AstarteType;
use astarte_sdk::{registration, AstarteError, AstarteSdk, Clientbound};
use async_trait::async_trait;
use serde::Serialize;

use crate::data::Publisher;
use crate::error::DeviceManagerError;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;
use crate::{get_hardware_id_from_dbus, DeviceManagerOptions};

#[derive(Clone)]
pub struct Astarte {
    pub device_sdk: AstarteSdk,
}

#[async_trait]
impl Publisher for Astarte {
    async fn send_object<T: 'static>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), AstarteError>
    where
        T: Serialize + Send,
    {
        self.device_sdk
            .send_object(interface_name, interface_path, data)
            .await
    }

    async fn send(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: AstarteType,
    ) -> Result<(), AstarteError> {
        self.device_sdk
            .send(interface_name, interface_path, data)
            .await
    }

    async fn on_event(&mut self) -> Result<Clientbound, AstarteError> {
        self.device_sdk.poll().await
    }
}

impl Astarte {
    pub async fn new(sdk_options: AstarteOptions) -> Result<Astarte, DeviceManagerError> {
        let device = AstarteSdk::new(&sdk_options).await?;
        Ok(Astarte { device_sdk: device })
    }
}

pub async fn astarte_map_options(
    opts: &DeviceManagerOptions,
) -> Result<AstarteOptions, DeviceManagerError> {
    let device_id: String = get_device_id(opts.device_id.clone()).await?;
    let store_directory = opts.store_directory.to_owned();
    let credentials_secret: String = get_credentials_secret(
        &device_id,
        &opts,
        FileStateRepository::new(store_directory, format!("credentials_{}.json", device_id)),
    )
    .await?;

    let mut sdk_options = AstarteOptions::new(
        &opts.realm,
        &device_id,
        &credentials_secret,
        &opts.pairing_url,
    );

    if Some(true) == opts.astarte_ignore_ssl {
        sdk_options.ignore_ssl_errors();
    }

    Ok(sdk_options
        .interface_directory(&opts.interfaces_directory)?
        .build())
}

async fn get_device_id(opt_device_id: Option<String>) -> Result<String, DeviceManagerError> {
    if let Some(device_id) = opt_device_id {
        if !device_id.is_empty() {
            return Ok(device_id);
        }
    }

    get_hardware_id_from_dbus().await
}

async fn get_credentials_secret(
    device_id: &str,
    opts: &DeviceManagerOptions,
    cred_state_repo: impl StateRepository<String>,
) -> Result<String, DeviceManagerError> {
    if let Some(secret) = opts.credentials_secret.clone() {
        if !secret.is_empty() {
            return Ok(secret);
        }
    }
    if cred_state_repo.exists() {
        get_credentials_secret_from_persistence(cred_state_repo)
    } else if let Some(token) = opts.pairing_token.clone() {
        get_credentials_secret_from_registration(device_id, &token, opts, cred_state_repo).await
    } else {
        Err(DeviceManagerError::FatalError(
            "Missing arguments".to_string(),
        ))
    }
}

fn get_credentials_secret_from_persistence(
    cred_state_repo: impl StateRepository<String>,
) -> Result<String, DeviceManagerError> {
    Ok(cred_state_repo.read().expect("Unable to read secret"))
}

async fn get_credentials_secret_from_registration(
    device_id: &str,
    token: &str,
    opts: &DeviceManagerOptions,
    cred_state_repo: impl StateRepository<String>,
) -> Result<String, DeviceManagerError> {
    let registration =
        registration::register_device(token, &opts.pairing_url, &opts.realm, &device_id).await;
    if let Ok(credentials_secret) = registration {
        cred_state_repo
            .write(&credentials_secret)
            .expect("Unable to write secret");
        Ok(credentials_secret)
    } else {
        Err(DeviceManagerError::FatalError("Pairing error".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::MockStateRepository;
    use crate::{DeviceManagerError, DeviceManagerOptions};

    use crate::data::astarte::{
        get_credentials_secret, get_credentials_secret_from_registration, get_device_id,
    };

    #[tokio::test]
    async fn device_id_test() {
        assert_eq!(
            get_device_id(Some("target".to_string())).await.unwrap(),
            "target".to_string()
        );
    }

    #[tokio::test]
    async fn credentials_secret_test() {
        let state_mock = MockStateRepository::<String>::new();
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: None,
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
            telemetry_config: vec![],
        };
        assert_eq!(
            get_credentials_secret("device_id", &options, state_mock)
                .await
                .unwrap(),
            "credentials_secret".to_string()
        );
    }

    #[tokio::test]
    async fn not_enough_arguments_credentials_secret_test() {
        let mut state_mock = MockStateRepository::<String>::new();
        state_mock.expect_exists().returning(|| false);
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: None,
            credentials_secret: None,
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
            telemetry_config: vec![],
        };
        assert!(get_credentials_secret("device_id", &options, state_mock)
            .await
            .is_err());
    }

    #[tokio::test]
    #[should_panic(expected = "Unable to read secret: FatalError(\"\")")]
    async fn get_credentials_secret_persistence_fail() {
        let mut state_mock = MockStateRepository::<String>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock
            .expect_read()
            .returning(move || Err(DeviceManagerError::FatalError("".to_owned())));

        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_owned()),
            credentials_secret: None,
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
            telemetry_config: vec![],
        };

        assert!(get_credentials_secret("device_id", &options, state_mock)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn get_credentials_secret_persistence_success() {
        let mut state_mock = MockStateRepository::<String>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock
            .expect_read()
            .returning(move || Ok("cred_secret".to_owned()));

        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_owned()),
            credentials_secret: None,
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
            telemetry_config: vec![],
        };

        assert!(get_credentials_secret("device_id", &options, state_mock)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn get_credentials_secret_from_registration_fail() {
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_owned()),
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "./".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
            telemetry_config: vec![],
        };

        let state_mock = MockStateRepository::<String>::new();
        let cred_result =
            get_credentials_secret_from_registration("", "", &options, state_mock).await;
        assert!(cred_result.is_err());
        match cred_result.err().unwrap() {
            DeviceManagerError::FatalError(val) => {
                assert_eq!(val, "Pairing error".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }
}
