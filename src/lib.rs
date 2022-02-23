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

use astarte_sdk::AstarteSdk;
pub mod error;
mod telemetry;

use astarte_sdk::builder::AstarteOptions;
use error::DeviceManagerError;

pub struct DeviceManagerOptions {
    pub realm: String,
    pub device_id: String,
    pub credentials_secret: String,
    pub pairing_url: String,
    pub interface_json_path: String,
}
pub struct DeviceManager {
    sdk: AstarteSdk,
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

        let device = astarte_sdk::AstarteSdk::new(&sdk_options).await?;

        Ok(Self { sdk: device })
    }

    pub async fn run(&mut self) {
        let w = self.sdk.clone();
        tokio::task::spawn(async move {
            loop {
                let systatus = telemetry::systemstatus::get_system_status().unwrap();

                w.send_object(
                    "io.edgehog.devicemanager.SystemStatus",
                    "/systemStatus/",
                    systatus,
                )
                .await
                .unwrap();

                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });

        loop {
            match self.sdk.poll().await {
                Ok(data) => {
                    println!("incoming: {:?}", data);
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

        for i in telemetry::osinfo::get_os_info()? {
            device
                .send("io.edgehog.devicemanager.OSInfo", &i.0, i.1)
                .await?;
        }

        for i in telemetry::hardwareinfo::get_hardware_info()? {
            device
                .send("io.edgehog.devicemanager.HardwareInfo", &i.0, i.1)
                .await?;
        }

        Ok(())
    }
}
