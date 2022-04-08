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
use astarte_sdk::{Aggregation, AstarteSdk};
use error::DeviceManagerError;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

mod commands;
pub mod error;
mod ota_handler;
mod power_management;
mod rauc;
mod telemetry;

pub struct DeviceManagerOptions {
    pub realm: String,
    pub device_id: String,
    pub credentials_secret: String,
    pub pairing_url: String,
    pub interface_json_path: String,
    pub state_file_path: String,
    pub download_file_path: String,
}
pub struct DeviceManager {
    sdk: AstarteSdk,
    //we pass the ota event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<HashMap<String, AstarteType>>,
    telemetry: Arc<telemetry::Telemetry>,
}

impl DeviceManager {
    pub async fn new(opts: DeviceManagerOptions) -> Result<DeviceManager, DeviceManagerError> {
        let sdk_options = AstarteOptions::new(
            &opts.realm,
            &opts.device_id,
            &opts.credentials_secret,
            &opts.pairing_url,
        )
        .interface_directory(&opts.interface_json_path)?
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

        let telemetry_default_config = vec![crate::telemetry::TelemetryInterfaceConfig {
            interface_name: "io.edgehog.devicemanager.SystemStatus".to_owned(),
            enabled: true,
            period: 10,
        }];

        let tel = telemetry::Telemetry::from_default_config(telemetry_default_config);

        Ok(Self {
            sdk: device,
            telemetry: Arc::new(tel),
            ota_event_channel: tx,
        })
    }

    pub async fn run(&mut self) {
        let w = self.sdk.clone();

        let tel = self.telemetry.clone();
        tokio::task::spawn(async move {
            tel.run_telemetry(w).await;
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

                        (
                            "io.edgehog.devicemanager.config.Telemetry",
                            ["request", interface_name, endpoint],
                            Aggregation::Individual(data),
                        ) => {
                            self.telemetry
                                .telemetry_config_event(interface_name, endpoint, data)
                                .await
                        }

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
