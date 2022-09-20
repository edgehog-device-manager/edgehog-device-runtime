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

use astarte_sdk::types::AstarteType;
use astarte_sdk::{Aggregation, Clientbound};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::sync::mpsc::{Receiver, Sender};

use device::DeviceProxy;
use error::DeviceManagerError;

use crate::data::Publisher;
use crate::ota::ota_handler::OTAHandler;

mod commands;
pub mod data;
mod device;
pub mod error;
mod ota;
mod power_management;
mod repository;
mod telemetry;
pub mod wrapper;

#[derive(Debug, Deserialize, Clone)]
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

pub struct DeviceManager<T: Publisher + Clone> {
    publisher: T,
    //we pass all Astarte event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<Clientbound>,
    data_event_channel: Sender<Clientbound>,
}

impl<T: Publisher + Clone + 'static> DeviceManager<T> {
    pub async fn new(
        opts: DeviceManagerOptions,
        publisher: T,
    ) -> Result<DeviceManager<T>, DeviceManagerError> {
        wrapper::systemd::systemd_notify_status("Initializing");
        info!("Starting");

        let ota_handler = OTAHandler::new(&opts).await?;

        ota_handler.ensure_pending_ota_response(&publisher).await?;

        let (ota_tx, ota_rx) = tokio::sync::mpsc::channel(1);
        let (data_tx, data_rx) = tokio::sync::mpsc::channel(32);

        let device_runtime = Self {
            publisher,
            ota_event_channel: ota_tx,
            data_event_channel: data_tx,
        };

        device_runtime.init_ota_event(ota_handler, ota_rx);
        device_runtime.init_data_event(data_rx);
        Ok(device_runtime)
    }

    fn init_ota_event(
        &self,
        mut ota_handler: OTAHandler<'static>,
        mut ota_rx: Receiver<Clientbound>,
    ) {
        let astarte_client_clone = self.publisher.clone();
        tokio::spawn(async move {
            while let Some(clientbound) = ota_rx.recv().await {
                match (
                    clientbound
                        .path
                        .trim_matches('/')
                        .split('/')
                        .collect::<Vec<&str>>()
                        .as_slice(),
                    &clientbound.data,
                ) {
                    (["request"], Aggregation::Object(data)) => ota_handler
                        .ota_event(&astarte_client_clone, data.clone())
                        .await
                        .ok(),
                    _ => {
                        warn!("Receiving data from an unknown path/interface: {clientbound:?}");
                        Some(())
                    }
                };
            }
        });
    }

    fn init_data_event(&self, mut data_rx: Receiver<Clientbound>) {
        tokio::spawn(async move {
            while let Some(clientbound) = data_rx.recv().await {
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
                        "io.edgehog.devicemanager.Commands",
                        ["request"],
                        Aggregation::Individual(AstarteType::String(command)),
                    ) => commands::execute_command(command),
                    _ => {
                        warn!("Receiving data from an unknown path/interface: {clientbound:?}");
                    }
                }
            }
        });
    }

    pub async fn run(&mut self) {
        wrapper::systemd::systemd_notify_status("Running");
        let w = self.publisher.clone();
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
            match self.publisher.on_event().await {
                Ok(clientbound) => {
                    debug!("incoming: {:?}", clientbound);

                    match clientbound.interface.as_str() {
                        "io.edgehog.devicemanager.OTARequest" => {
                            self.ota_event_channel.send(clientbound).await.unwrap()
                        }
                        _ => {
                            self.data_event_channel.send(clientbound).await.unwrap();
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
        let device = &self.publisher;

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
        for (disk_name, storage) in disks {
            device
                .send_object(
                    "io.edgehog.devicemanager.StorageUsage",
                    format!("/{}", disk_name).as_str(),
                    storage,
                )
                .await?;
        }

        for wifi_scan_result in telemetry::wifi_scan::get_wifi_scan_results()? {
            device
                .send_object(
                    "io.edgehog.devicemanager.WiFiScanResults",
                    "/ap",
                    wifi_scan_result,
                )
                .await?;
        }

        Ok(())
    }
}

pub async fn get_hardware_id_from_dbus() -> Result<String, DeviceManagerError> {
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

#[cfg(test)]
mod tests {
    use crate::data::astarte::{astarte_map_options, Astarte};
    use crate::data::MockPublisher;
    use crate::telemetry::hardware_info::get_hardware_info;
    use crate::telemetry::net_if_properties::get_network_interface_properties;
    use crate::telemetry::os_info::get_os_info;
    use crate::telemetry::runtime_info::get_runtime_info;
    use crate::telemetry::storage_usage::{get_storage_usage, DiskUsage};
    use crate::telemetry::system_info::get_system_info;
    use crate::{DeviceManager, DeviceManagerOptions};
    use astarte_sdk::types::AstarteType;

    impl Clone for MockPublisher {
        fn clone(&self) -> Self {
            MockPublisher::new()
        }

        fn clone_from(&mut self, _: &Self) {}
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

        let astarte_options = astarte_map_options(&options).await.unwrap();
        let astarte = Astarte::new(astarte_options).await.unwrap();
        let dm = DeviceManager::new(options, astarte).await;

        assert!(dm.is_ok());
    }

    #[tokio::test]
    async fn device_manager_new_success() {
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

        let dm = DeviceManager::new(options, MockPublisher::new()).await;
        assert!(dm.is_ok());
    }

    #[tokio::test]
    async fn send_initial_telemetry_success() {
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

        let os_info = get_os_info().unwrap();
        let mut publisher = MockPublisher::new();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.OSInfo"
                        && os_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let hardware_info = get_hardware_info().unwrap();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.HardwareInfo"
                        && hardware_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let runtime_info = get_runtime_info().unwrap();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.RuntimeInfo"
                        && runtime_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let storage_usage = get_storage_usage().unwrap();
        publisher
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &DiskUsage| {
                    interface_name == "io.edgehog.devicemanager.StorageUsage"
                        && storage_usage.contains_key(&interface_path[1..])
                },
            )
            .returning(|_: &str, _: &str, _: DiskUsage| Ok(()));

        let network_iface_props = get_network_interface_properties().await.unwrap();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.NetworkInterfaceProperties"
                        && network_iface_props.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let system_info = get_system_info().unwrap();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.SystemInfo"
                        && system_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let dm = DeviceManager::new(options, publisher).await;
        assert!(dm.is_ok());

        let telemetry_result = dm.unwrap().send_initial_telemetry().await;
        assert!(telemetry_result.is_ok());
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
