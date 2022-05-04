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

use std::collections::HashMap;

use astarte_sdk::builder::AstarteOptions;
use astarte_sdk::types::AstarteType;
use astarte_sdk::{registration, Aggregation, AstarteSdk};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::sync::mpsc::Sender;

use device::DeviceProxy;
use error::DeviceManagerError;

use crate::astarte::Astarte;
use crate::data::astarte;
use crate::ota::ota_handler::OTAHandler;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;

mod commands;
mod data;
mod device;
pub mod error;
mod ota;
mod power_management;
mod repository;
mod telemetry;
pub mod wrapper;

#[derive(Debug, Deserialize)]
pub struct DeviceManagerOptions {
    pub realm: String,
    pub device_id: Option<String>,
    pub credentials_secret: Option<String>,
    pub pairing_url: String,
    pub pairing_token: Option<String>,
    pub interfaces_directory: String,
    pub store_directory: String,
    pub download_directory: String,
    pub astarte_ignore_ssl: Option<bool>,
}

pub struct DeviceManager {
    sdk: AstarteSdk,
    //we pass the ota event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<HashMap<String, AstarteType>>,
}

impl DeviceManager {
    pub async fn new(opts: DeviceManagerOptions) -> Result<DeviceManager, DeviceManagerError> {
        let device_id: String = get_device_id(opts.device_id.clone()).await?;
        let credentials_secret: String = get_credentials_secret(
            &device_id,
            &opts,
            FileStateRepository::new(
                opts.store_directory.clone(),
                format!("credentials_{}.json", device_id),
            ),
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

        let sdk_options = sdk_options
            .interface_directory(&opts.interfaces_directory)?
            .build();
        info!("Starting");

        wrapper::systemd::systemd_notify_status("Initializing");
        let astarte_client = Astarte::new(&sdk_options).await?;

        let mut ota_handler = OTAHandler::new(&opts).await?;

        ota_handler
            .ensure_pending_ota_response(&astarte_client)
            .await?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let astarte_client_clone = astarte_client.clone();
        tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                ota_handler
                    .ota_event(&astarte_client_clone, data)
                    .await
                    .ok();
            }
        });

        Ok(Self {
            sdk: astarte_client.device_sdk,
            ota_event_channel: tx,
        })
    }

    pub async fn run(&mut self) {
        wrapper::systemd::systemd_notify_status("Running");
        let w = self.sdk.clone();
        tokio::task::spawn(async move {
            loop {
                let systatus = telemetry::system_status::get_system_status().unwrap();

                w.send_object(
                    "io.edgehog.devicemanager.SystemStatus",
                    "/systemStatus",
                    systatus,
                )
                .await
                .unwrap();

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        loop {
            match self.sdk.poll().await {
                Ok(clientbound) => {
                    debug!("incoming: {:?}", clientbound);

                    match (
                        clientbound.interface.as_str(),
                        clientbound
                            .path
                            .trim_matches('/')
                            .split('/')
                            .collect::<Vec<&str>>()
                            .as_slice(),
                        &clientbound.data,
                    ) {
                        (
                            "io.edgehog.devicemanager.OTARequest",
                            ["request"],
                            Aggregation::Object(data),
                        ) => self.ota_event_channel.send(data.clone()).await.unwrap(),

                        (
                            "io.edgehog.devicemanager.Commands",
                            ["request"],
                            Aggregation::Individual(AstarteType::String(command)),
                        ) => commands::execute_command(command),

                        _ => {
                            warn!("Receiving data from an unknown path/interface: {clientbound:?}");
                        }
                    }
                }
                Err(err) => log::error!("{:?}", err),
            }
        }
    }

    pub async fn init(&self) -> Result<(), DeviceManagerError> {
        wrapper::systemd::systemd_notify_status("Sending initial telemetry");
        self.send_initial_telemetry().await?;

        Ok(())
    }

    pub async fn send_initial_telemetry(&self) -> Result<(), DeviceManagerError> {
        let device = &self.sdk;

        let data = [
            (
                "io.edgehog.devicemanager.OSInfo",
                telemetry::os_info::get_os_info()?,
            ),
            (
                "io.edgehog.devicemanager.HardwareInfo",
                telemetry::hardware_info::get_hardware_info()?,
            ),
            (
                "io.edgehog.devicemanager.RuntimeInfo",
                telemetry::runtime_info::get_runtime_info()?,
            ),
            (
                "io.edgehog.devicemanager.NetworkInterfaceProperties",
                telemetry::net_if_properties::get_network_interface_properties().await?,
            ),
            (
                "io.edgehog.devicemanager.SystemInfo",
                telemetry::system_info::get_system_info()?,
            ),
        ];

        for (ifc, fields) in data {
            for (path, data) in fields {
                device.send(ifc, &path, data).await?;
            }
        }

        let disks = telemetry::storage_usage::get_storage_usage()?;
        for (disk_name, storage) in &disks {
            device
                .send_object(
                    "io.edgehog.devicemanager.StorageUsage",
                    format!("/{}", disk_name).as_str(),
                    storage,
                )
                .await?;
        }
        Ok(())
    }
}

async fn get_device_id(opt_device_id: Option<String>) -> Result<String, DeviceManagerError> {
    if let Some(device_id) = opt_device_id {
        Ok(device_id)
    } else {
        get_hardware_id_from_dbus().await
    }
}

async fn get_hardware_id_from_dbus() -> Result<String, DeviceManagerError> {
    let connection = zbus::Connection::system().await?;
    let proxy = DeviceProxy::new(&connection).await?;
    let hardware_id: String = proxy.get_hardware_id("").await?;
    if hardware_id.is_empty() {
        return Err(DeviceManagerError::FatalError(
            "No hardware id provided".to_string(),
        ));
    }
    Ok(hardware_id)
}

async fn get_credentials_secret(
    device_id: &str,
    opts: &DeviceManagerOptions,
    cred_state_repo: impl StateRepository<String>,
) -> Result<String, DeviceManagerError> {
    if let Some(secret) = opts.credentials_secret.clone() {
        Ok(secret)
    } else if cred_state_repo.exists() {
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
    use crate::{
        get_credentials_secret, get_credentials_secret_from_registration, get_device_id,
        DeviceManager, DeviceManagerError, DeviceManagerOptions,
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
        };
        assert!(get_credentials_secret("device_id", &options, state_mock)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn device_option_empty_interface_path_fail() {
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_string()),
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
        };
        let mut dm = DeviceManager::new(options).await;

        assert!(dm.is_err());
    }

    #[tokio::test]
    #[should_panic]
    async fn device_new_sdk_panic_fail() {
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_string()),
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "./".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
        };
        let mut dm = DeviceManager::new(options).await;

        assert!(dm.is_ok());
    }

    #[tokio::test]
    async fn get_credentials_secret_from_registration_fail() {
        let options = DeviceManagerOptions {
            realm: "".to_string(),
            device_id: Some("device_id".to_string()),
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_url: "".to_string(),
            pairing_token: None,
            interfaces_directory: "./".to_string(),
            store_directory: "".to_string(),
            download_directory: "".to_string(),
            astarte_ignore_ssl: Some(false),
        };
        let mut state_mock = MockStateRepository::<String>::new();
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

#[cfg(not(tarpaulin))]
#[cfg(feature = "e2e_test")]
pub mod e2e_test {
    use crate::{telemetry, DeviceManagerError};
    use astarte_sdk::types::AstarteType;
    use std::collections::HashMap;

    pub fn get_os_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::os_info::get_os_info()
    }

    pub fn get_hardware_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::hardware_info::get_hardware_info()
    }

    pub fn get_runtime_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::runtime_info::get_runtime_info()
    }
}
