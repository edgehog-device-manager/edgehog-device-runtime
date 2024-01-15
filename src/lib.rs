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

use std::path::PathBuf;
use std::sync::Arc;

use astarte_device_sdk::types::AstarteType;
use astarte_device_sdk::{Aggregation, AstarteDeviceDataEvent};
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::RwLock;

use crate::data::{Publisher, Subscriber};
use crate::error::DeviceManagerError;
use crate::ota::ota_handler::OtaHandler;
use crate::telemetry::{TelemetryMessage, TelemetryPayload};

mod commands;
pub mod data;
mod device;
pub mod error;
mod led_behavior;
mod ota;
mod power_management;
pub mod repository;
#[cfg(feature = "systemd")]
pub mod systemd_wrapper;
mod telemetry;

const MAX_OTA_OPERATION: usize = 2;

#[derive(Deserialize, Debug, Clone)]
pub enum AstarteLibrary {
    #[serde(rename = "astarte-device-sdk")]
    AstarteDeviceSDK,
    #[serde(rename = "astarte-message-hub")]
    AstarteMessageHub,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DeviceManagerOptions {
    pub astarte_library: AstarteLibrary,
    pub astarte_device_sdk: Option<data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions>,
    pub astarte_message_hub: Option<data::astarte_message_hub_node::AstarteMessageHubOptions>,
    pub interfaces_directory: PathBuf,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
    pub telemetry_config: Option<Vec<telemetry::TelemetryInterfaceConfig>>,
}

pub struct DeviceManager<T: Publisher + Subscriber + Clone> {
    publisher: T,
    //we pass all Astarte event through a channel, to avoid blocking the main loop
    ota_event_channel: Sender<AstarteDeviceDataEvent>,
    data_event_channel: Sender<AstarteDeviceDataEvent>,
    telemetry: Arc<RwLock<telemetry::Telemetry>>,
}

impl<T: Publisher + Subscriber + Clone + 'static> DeviceManager<T> {
    pub async fn new(
        opts: DeviceManagerOptions,
        publisher: T,
    ) -> Result<DeviceManager<T>, DeviceManagerError> {
        #[cfg(feature = "systemd")]
        systemd_wrapper::systemd_notify_status("Initializing");

        info!("Starting");

        let ota_handler = OtaHandler::new(&opts).await?;

        ota_handler.ensure_pending_ota_is_done(&publisher).await?;

        let (ota_tx, ota_rx) = channel(MAX_OTA_OPERATION);
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
        ota_handler: OtaHandler,
        mut ota_rx: Receiver<AstarteDeviceDataEvent>,
    ) {
        let publisher = self.publisher.clone();
        let ota_handler = Arc::new(ota_handler);
        tokio::spawn(async move {
            while let Some(data_event) = ota_rx.recv().await {
                match (
                    data_event
                        .path
                        .trim_matches('/')
                        .split('/')
                        .collect::<Vec<&str>>()
                        .as_slice(),
                    &data_event.data,
                ) {
                    (["request"], Aggregation::Object(data)) => {
                        let publisher = publisher.clone();
                        let data = data.clone();
                        let ota_handler = ota_handler.clone();
                        tokio::spawn(async move {
                            let _ = ota_handler.ota_event(&publisher, data).await;
                        });
                    }
                    _ => {
                        warn!("Receiving data from an unknown path/interface: {data_event:?}");
                    }
                }
            }
        });
    }

    fn init_data_event(&self, mut data_rx: Receiver<AstarteDeviceDataEvent>) {
        let self_telemetry = self.telemetry.clone();
        tokio::spawn(async move {
            while let Some(data_event) = data_rx.recv().await {
                match (
                    data_event.interface.as_str(),
                    data_event
                        .path
                        .trim_matches('/')
                        .split('/')
                        .collect::<Vec<&str>>()
                        .as_slice(),
                    &data_event.data,
                ) {
                    (
                        "io.edgehog.devicemanager.Commands",
                        ["request"],
                        Aggregation::Individual(AstarteType::String(command)),
                    ) => commands::execute_command(command).await,
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
                    (
                        "io.edgehog.devicemanager.LedBehavior",
                        [led_id, "behavior"],
                        Aggregation::Individual(AstarteType::String(behavior)),
                    ) => {
                        tokio::spawn(led_behavior::set_behavior(
                            led_id.to_string(),
                            behavior.clone(),
                        ));
                    }
                    _ => {
                        warn!("Receiving data from an unknown path/interface: {data_event:?}");
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
        #[cfg(feature = "systemd")]
        systemd_wrapper::systemd_notify_status("Running");

        let tel_clone = self.telemetry.clone();
        tokio::task::spawn(async move {
            tel_clone.write().await.run_telemetry().await;
        });

        loop {
            match self.publisher.on_event().await {
                Ok(data_event) => {
                    debug!("incoming: {:?}", data_event);

                    match data_event.interface.as_str() {
                        "io.edgehog.devicemanager.OTARequest" => {
                            self.ota_event_channel.send(data_event).await.unwrap()
                        }
                        _ => {
                            self.data_event_channel.send(data_event).await.unwrap();
                        }
                    }
                }
                Err(err) => log::error!("{:?}", err),
            }
        }
    }

    pub async fn init(&self) -> Result<(), DeviceManagerError> {
        #[cfg(feature = "systemd")]
        systemd_wrapper::systemd_notify_status("Sending initial telemetry");

        self.send_initial_telemetry().await?;

        Ok(())
    }

    pub async fn send_initial_telemetry(&self) -> Result<(), DeviceManagerError> {
        let device = &self.publisher;

        let data = [
            (
                "io.edgehog.devicemanager.OSInfo",
                telemetry::os_info::get_os_info().await?,
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
                telemetry::base_image::get_base_image().await?,
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

#[cfg(not(tarpaulin))]
#[cfg(feature = "e2e_test")]
pub mod e2e_test {
    use crate::{telemetry, DeviceManagerError};
    use astarte_device_sdk::types::AstarteType;
    use std::collections::HashMap;

    pub async fn get_os_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::os_info::get_os_info().await
    }

    pub fn get_hardware_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::hardware_info::get_hardware_info()
    }

    pub fn get_runtime_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
        telemetry::runtime_info::get_runtime_info()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use astarte_device_sdk::types::AstarteType;
    use astarte_device_sdk::{AstarteAggregate, AstarteDeviceDataEvent};
    use async_trait::async_trait;
    use mockall::mock;

    use crate::data::astarte_device_sdk_lib::{AstarteDeviceSdkConfigOptions, AstarteDeviceSdkLib};
    use crate::data::{Publisher, Subscriber};
    use crate::telemetry::base_image::get_base_image;
    use crate::telemetry::battery_status::{get_battery_status, BatteryStatus};
    use crate::telemetry::hardware_info::get_hardware_info;
    use crate::telemetry::net_if_properties::get_network_interface_properties;
    use crate::telemetry::os_info::get_os_info;
    use crate::telemetry::runtime_info::get_runtime_info;
    use crate::telemetry::storage_usage::{get_storage_usage, DiskUsage};
    use crate::telemetry::system_info::get_system_info;
    use crate::telemetry::system_status::{get_system_status, SystemStatus};
    use crate::{
        AstarteLibrary, DeviceManager, DeviceManagerOptions, TelemetryMessage, TelemetryPayload,
    };

    mock! {
        AstarteHandler { }

        impl Clone for AstarteHandler {
            fn clone(&self) -> Self;
        }

         #[async_trait]
        impl Publisher for AstarteHandler {
            async fn send_object<T: 'static>(
                &self,
                interface_name: &str,
                interface_path: &str,
                data: T,
            ) -> Result<(), astarte_device_sdk::Error>
            where
                T: AstarteAggregate + Send;

            async fn send(
                &self,
                interface_name: &str,
                interface_path: &str,
                data: AstarteType,
            ) -> Result<(), astarte_device_sdk::Error>;
        }

        #[async_trait]
        impl Subscriber for AstarteHandler {
            async fn on_event(&mut self) -> Result<AstarteDeviceDataEvent, astarte_device_sdk::Error>;
        }
    }

    #[tokio::test]
    #[should_panic]
    async fn device_new_sdk_panic_fail() {
        let options = DeviceManagerOptions {
            astarte_library: AstarteLibrary::AstarteDeviceSDK,
            astarte_device_sdk: Some(AstarteDeviceSdkConfigOptions {
                realm: "".to_string(),
                device_id: Some("device_id".to_string()),
                credentials_secret: Some("credentials_secret".to_string()),
                pairing_url: "".to_string(),
                pairing_token: None,
                ignore_ssl: false,
            }),
            astarte_message_hub: None,
            interfaces_directory: PathBuf::new(),
            store_directory: PathBuf::new(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let lib = AstarteDeviceSdkLib::connect(&options).await.unwrap();
        let dm = DeviceManager::new(options, lib).await;

        assert!(dm.is_ok());
    }

    #[tokio::test]
    async fn device_manager_new_success() {
        let options = DeviceManagerOptions {
            astarte_library: AstarteLibrary::AstarteDeviceSDK,
            astarte_device_sdk: Some(AstarteDeviceSdkConfigOptions {
                realm: "".to_string(),
                device_id: Some("device_id".to_string()),
                credentials_secret: Some("credentials_secret".to_string()),
                pairing_url: "".to_string(),
                pairing_token: None,
                ignore_ssl: false,
            }),
            astarte_message_hub: None,
            interfaces_directory: PathBuf::new(),
            store_directory: PathBuf::new(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let mut mock_astarte_handler = MockAstarteHandler::new();
        mock_astarte_handler
            .expect_clone()
            .returning(MockAstarteHandler::new);

        let dm = DeviceManager::new(options, mock_astarte_handler).await;
        if let Err(ref e) = dm {
            eprintln!("{:?}", e);
        }
        assert!(dm.is_ok());
    }

    #[tokio::test]
    async fn send_initial_telemetry_success() {
        let options = DeviceManagerOptions {
            astarte_library: AstarteLibrary::AstarteDeviceSDK,
            astarte_device_sdk: Some(AstarteDeviceSdkConfigOptions {
                realm: "".to_string(),
                device_id: Some("device_id".to_string()),
                credentials_secret: Some("credentials_secret".to_string()),
                pairing_url: "".to_string(),
                pairing_token: None,
                ignore_ssl: false,
            }),
            astarte_message_hub: None,
            interfaces_directory: PathBuf::new(),
            store_directory: PathBuf::new(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let os_info = get_os_info().await.expect("failed to get os info");
        let mut mock_astarte_handler = MockAstarteHandler::new();

        mock_astarte_handler
            .expect_clone()
            .returning(MockAstarteHandler::new);
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.OSInfo"
                        && os_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let hardware_info = get_hardware_info().unwrap();
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.HardwareInfo"
                        && hardware_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let runtime_info = get_runtime_info().unwrap();
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.RuntimeInfo"
                        && runtime_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let storage_usage = get_storage_usage().unwrap();
        mock_astarte_handler
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &DiskUsage| {
                    interface_name == "io.edgehog.devicemanager.StorageUsage"
                        && storage_usage.contains_key(&interface_path[1..])
                },
            )
            .returning(|_: &str, _: &str, _: DiskUsage| Ok(()));

        let network_iface_props = get_network_interface_properties().await.unwrap();
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.NetworkInterfaceProperties"
                        && network_iface_props.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let system_info = get_system_info().unwrap();
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.SystemInfo"
                        && system_info.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let base_image = get_base_image().await.expect("failed to get base image");
        mock_astarte_handler
            .expect_send()
            .withf(
                move |interface_name: &str, interface_path: &str, data: &AstarteType| {
                    interface_name == "io.edgehog.devicemanager.BaseImage"
                        && base_image.get(interface_path).unwrap() == data
                },
            )
            .returning(|_: &str, _: &str, _: AstarteType| Ok(()));

        let dm = DeviceManager::new(options, mock_astarte_handler).await;
        assert!(dm.is_ok());

        let telemetry_result = dm.unwrap().send_initial_telemetry().await;
        assert!(telemetry_result.is_ok());
    }

    #[tokio::test]
    async fn send_telemetry_success() {
        let system_status = get_system_status().unwrap();
        let mut mock_astarte_handler: MockAstarteHandler = MockAstarteHandler::new();
        mock_astarte_handler
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &SystemStatus| {
                    interface_name == "io.edgehog.devicemanager.SystemStatus"
                        && interface_path == "/systemStatus"
                },
            )
            .returning(|_: &str, _: &str, _: SystemStatus| Ok(()));

        let storage_usage = get_storage_usage().unwrap();
        mock_astarte_handler
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &DiskUsage| {
                    interface_name == "io.edgehog.devicemanager.StorageUsage"
                        && storage_usage.contains_key(&interface_path[1..])
                },
            )
            .returning(|_: &str, _: &str, _: DiskUsage| Ok(()));

        let battery_status = get_battery_status().await.unwrap();
        mock_astarte_handler
            .expect_send_object()
            .withf(
                move |interface_name: &str, interface_path: &str, _: &BatteryStatus| {
                    interface_name == "io.edgehog.devicemanager.BatteryStatus"
                        && battery_status.contains_key(&interface_path[1..])
                },
            )
            .returning(|_: &str, _: &str, _: BatteryStatus| Ok(()));

        DeviceManager::<MockAstarteHandler>::send_telemetry(
            &mock_astarte_handler,
            TelemetryMessage {
                path: "".to_string(),
                payload: TelemetryPayload::SystemStatus(system_status),
            },
        )
        .await;
        for (path, payload) in get_storage_usage().unwrap() {
            DeviceManager::<MockAstarteHandler>::send_telemetry(
                &mock_astarte_handler,
                TelemetryMessage {
                    path,
                    payload: TelemetryPayload::StorageUsage(payload),
                },
            )
            .await;
        }
        for (path, payload) in get_battery_status().await.unwrap() {
            DeviceManager::<MockAstarteHandler>::send_telemetry(
                &mock_astarte_handler,
                TelemetryMessage {
                    path,
                    payload: TelemetryPayload::BatteryStatus(payload),
                },
            )
            .await;
        }
    }
}
