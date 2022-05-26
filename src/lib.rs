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
use astarte_sdk::{registration, Aggregation, AstarteSdk};
use device::DeviceProxy;
use error::DeviceManagerError;
use log::{debug, info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use tokio::sync::mpsc::Sender;

mod commands;
mod device;
pub mod error;
mod ota_handler;
mod power_management;
mod rauc;
mod repository;
mod telemetry;

#[derive(Debug, Deserialize)]
pub struct DeviceManagerOptions {
    pub realm: String,
    pub device_id: Option<String>,
    pub credentials_secret: Option<String>,
    pub pairing_url: String,
    pub pairing_token: Option<String>,
    pub interfaces_directory: String,
    pub state_file: String,
    pub download_directory: String,
}

pub struct DeviceManager {
    sdk: AstarteSdk,
    //we pass the ota event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<HashMap<String, AstarteType>>,
}

impl DeviceManager {
    pub async fn new(opts: DeviceManagerOptions) -> Result<DeviceManager, DeviceManagerError> {
        let device_id: String = get_device_id(opts.device_id.clone()).await?;
        let credential_secret: String = get_credentials_secret(&device_id, &opts).await?;

        let sdk_options = AstarteOptions::new(
            &opts.realm,
            &device_id,
            &credential_secret,
            &opts.pairing_url,
        )
        .interface_directory(&opts.interfaces_directory)?
        .build();
        info!("Starting");

        let device = astarte_sdk::AstarteSdk::new(&sdk_options).await?;

        let mut ota_handler = ota_handler::OTAHandler::new(&opts).await?;

        ota_handler.ensure_pending_ota_response(&device).await?;

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let sdk_clone = device.clone();
        tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                ota_handler.ota_event(&sdk_clone, data).await.ok();
            }
        });

        Ok(Self {
            sdk: device,
            ota_event_channel: tx,
        })
    }

    pub async fn run(&mut self) {
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
        ];

        for (ifc, fields) in data {
            for (path, data) in fields {
                device.send(ifc, &path, data).await?;
            }
        }

        Ok(())
    }
}

async fn get_device_id(opt_device_id: Option<String>) -> Result<String, DeviceManagerError> {
    if let Some(device_id) = opt_device_id {
        Ok(device_id)
    } else {
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
}

async fn get_credentials_secret(
    device_id: &str,
    opts: &DeviceManagerOptions,
) -> Result<String, DeviceManagerError> {
    if let Some(secret) = opts.credentials_secret.clone() {
        Ok(secret)
    } else if Path::new(&format!("./{}.json", device_id)).exists() {
        let reader = File::open(&format!("./{}.json", device_id)).unwrap();
        Ok(serde_json::from_reader(reader).expect("Unable to read secret"))
    } else if let Some(token) = opts.pairing_token.clone() {
        let registration =
            registration::register_device(&token, &opts.pairing_url, &opts.realm, &device_id).await;
        if let Ok(credential_secret) = registration {
            let writer = File::create(&format!("./{}.json", device_id)).unwrap();
            serde_json::to_writer(writer, &credential_secret).expect("Unable to write secret");
            Ok(credential_secret)
        } else {
            Err(DeviceManagerError::FatalError("Pairing error".to_string()))
        }
    } else {
        Err(DeviceManagerError::FatalError(
            "Missing arguments".to_string(),
        ))
    }
}
