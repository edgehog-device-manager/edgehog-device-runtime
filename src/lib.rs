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

use std::sync::Arc;

use astarte_sdk::types::AstarteType;
use astarte_sdk::{Aggregation, Clientbound};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::RwLock;

use crate::data::Publisher;
use crate::device::DeviceProxy;
use crate::error::DeviceManagerError;
use crate::ota::ota_handler::OTAHandler;
use crate::telemetry::{TelemetryMessage, TelemetryPayload};

mod commands;
pub mod data;
mod device;
pub mod error;
mod ota;
mod power_management;
pub mod repository;
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
    pub telemetry_config: Option<Vec<telemetry::TelemetryInterfaceConfig>>,
}

pub struct DeviceManager<T: Publisher + Clone> {
    publisher: T,
    //we pass all Astarte event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<Clientbound>,
    data_event_channel: Sender<Clientbound>,
    telemetry: Arc<RwLock<telemetry::Telemetry>>,
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

        let (ota_tx, ota_rx) = channel(1);
        let (data_tx, data_rx) = channel(32);

        let (telemetry_tx, telemetry_rx) = channel(32);

        let tel = telemetry::Telemetry::from_default_config(
            opts.telemetry_config,
            telemetry_tx,
            opts.store_directory.clone(),
        )
        .await;

        let device_runtime = Self {
            publisher,
            ota_event_channel: ota_tx,
            data_event_channel: data_tx,
            telemetry: Arc::new(RwLock::new(tel)),
        };

        device_runtime.init_ota_event(ota_handler, ota_rx);
        device_runtime.init_data_event(data_rx);
        device_runtime.init_telemetry_event(telemetry_rx);
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
        let self_telemetry = self.telemetry.clone();
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
                    (
                        "io.edgehog.devicemanager.config.Telemetry",
                        ["request", interface_name, endpoint],
                        Aggregation::Individual(data),
                    ) => {
                        self_telemetry
                            .write()
                            .await
                            .telemetry_config_event(interface_name, endpoint, data)
                            .await;
                    }
                    _ => {
                        warn!("Receiving data from an unknown path/interface: {clientbound:?}");
                    }
                }
            }
        });
    }

    fn init_telemetry_event(&self, mut telemetry_rx: Receiver<TelemetryMessage>) {
        let publisher = self.publisher.clone();
        tokio::spawn(async move {
            while let Some(msg) = telemetry_rx.recv().await {
                Self::send_telemetry(&publisher, msg).await;
            }
        });
    }

    pub async fn run(&mut self) {
        wrapper::systemd::systemd_notify_status("Running");
        let tel_clone = self.telemetry.clone();
        tokio::task::spawn(async move {
            tel_clone.write().await.run_telemetry().await;
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
            (
                "io.edgehog.devicemanager.BaseImage",
                telemetry::base_image::get_base_image()?,
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

    async fn send_telemetry(publisher: &impl Publisher, msg: TelemetryMessage) {
        match msg.payload {
            TelemetryPayload::SystemStatus(data) => {
                let _ = publisher
                    .send_object(
                        "io.edgehog.devicemanager.SystemStatus",
                        "/systemStatus",
                        data,
                    )
                    .await;
            }
            TelemetryPayload::StorageUsage(data) => {
                let _ = publisher
                    .send_object(
                        "io.edgehog.devicemanager.StorageUsage",
                        format!("/{}", msg.path).as_str(),
                        data,
                    )
                    .await;
            }
            TelemetryPayload::BatteryStatus(data) => {
                let _ = publisher
                    .send_object(
                        "io.edgehog.devicemanager.BatteryStatus",
                        format!("/{}", msg.path).as_str(),
                        data,
                    )
                    .await;
            }
        };
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
    use crate::telemetry::base_image::get_base_image;
    use crate::telemetry::battery_status::{get_battery_status, BatteryStatus};
    use crate::telemetry::hardware_info::get_hardware_info;
    use crate::telemetry::net_if_properties::get_network_interface_properties;
    use crate::telemetry::os_info::get_os_info;
    use crate::telemetry::runtime_info::get_runtime_info;
    use crate::telemetry::storage_usage::{get_storage_usage, DiskUsage};
    use crate::telemetry::system_info::get_system_info;
    use crate::telemetry::system_status::{get_system_status, SystemStatus};
    use crate::{DeviceManager, DeviceManagerOptions, TelemetryMessage, TelemetryPayload};
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
            telemetry_config: Some(vec![]),
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
            telemetry_config: Some(vec![]),
        };

        let dm = DeviceManager::new(options, MockPublisher::new()).await;
        if let Err(ref e) = dm {
            println!("{:?}", e);
        }
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
            telemetry_config: Some(vec![]),
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

        let base_image = get_base_image().unwrap();
        publisher
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.BaseImage"
                        && base_image.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let dm = DeviceManager::new(options, publisher).await;
        assert!(dm.is_ok());

        let telemetry_result = dm.unwrap().send_initial_telemetry().await;
        assert!(telemetry_result.is_ok());
    }

    #[tokio::test]
    async fn send_telemetry_success() {
        let system_status = get_system_status().unwrap();
        let mut publisher: MockPublisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &SystemStatus| {
                    interface_name == "io.edgehog.devicemanager.SystemStatus"
                        && interface_path == "/systemStatus"
                },
            )
            .returning(|_: &str, _: &str, _: SystemStatus| Ok(()));

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

        let battery_status = get_battery_status().await.unwrap();
        publisher
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &BatteryStatus| {
                    interface_name == "io.edgehog.devicemanager.BatteryStatus"
                        && battery_status.contains_key(&interface_path[1..])
                },
            )
            .returning(|_: &str, _: &str, _: BatteryStatus| Ok(()));

        DeviceManager::<MockPublisher>::send_telemetry(
            &publisher,
            TelemetryMessage {
                path: "".to_string(),
                payload: TelemetryPayload::SystemStatus(system_status),
            },
        )
        .await;
        for (path, payload) in get_storage_usage().unwrap() {
            DeviceManager::<MockPublisher>::send_telemetry(
                &publisher,
                TelemetryMessage {
                    path,
                    payload: TelemetryPayload::StorageUsage(payload),
                },
            )
            .await;
        }
        for (path, payload) in get_battery_status().await.unwrap() {
            DeviceManager::<MockPublisher>::send_telemetry(
                &publisher,
                TelemetryMessage {
                    path,
                    payload: TelemetryPayload::BatteryStatus(payload),
                },
            )
            .await;
        }
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
