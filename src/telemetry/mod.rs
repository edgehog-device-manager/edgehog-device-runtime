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

use crate::error::DeviceManagerError;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;
use astarte_device_sdk::types::AstarteType;
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::broadcast::{channel, Receiver, Sender};
use tokio::sync::mpsc::Sender as MpscSender;
use tokio::sync::RwLock;
use tokio::task::spawn;
use tokio::time::interval;
use tokio::time::Duration;

pub(crate) mod base_image;
pub(crate) mod battery_status;
pub mod hardware_info;
pub(crate) mod net_if_properties;
pub mod os_info;
pub mod runtime_info;
pub(crate) mod storage_usage;
pub(crate) mod system_info;
pub(crate) mod system_status;
pub(crate) mod upower;
pub(crate) mod wifi_scan;

const TELEMETRY_PATH: &str = "telemetry.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryInterfaceConfig {
    pub interface_name: String,
    pub enabled: Option<bool>,
    pub period: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct TelemetryTaskConfig {
    default_enabled: Option<bool>,
    default_period: Option<u64>,
    override_enabled: Option<bool>,
    override_period: Option<u64>,
}

#[derive(Debug)]
pub struct Telemetry {
    telemetry_task_configs: Arc<RwLock<HashMap<String, TelemetryTaskConfig>>>,
    kill_switches: HashMap<String, Sender<()>>,
    communication_channel: MpscSender<TelemetryMessage>,
    store_directory: PathBuf,
}

pub enum TelemetryPayload {
    SystemStatus(crate::telemetry::system_status::SystemStatus),
    StorageUsage(crate::telemetry::storage_usage::DiskUsage),
    BatteryStatus(crate::telemetry::battery_status::BatteryStatus),
}

pub struct TelemetryMessage {
    pub path: String,
    pub payload: TelemetryPayload,
}

impl Telemetry {
    pub async fn from_default_config(
        cfg: Option<Vec<TelemetryInterfaceConfig>>,
        communication_channel: MpscSender<TelemetryMessage>,
        store_directory: PathBuf,
    ) -> Self {
        let cfg = match cfg {
            None => {
                return Telemetry {
                    telemetry_task_configs: Arc::new(Default::default()),
                    kill_switches: Default::default(),
                    communication_channel,
                    store_directory,
                }
            }
            Some(conf) => conf,
        };
        let mut telemetry_task_configs = HashMap::new();
        for c in cfg {
            telemetry_task_configs.insert(
                c.interface_name.clone(),
                TelemetryTaskConfig {
                    default_enabled: c.enabled,
                    default_period: c.period,
                    override_enabled: None,
                    override_period: None,
                },
            );
        }

        let telemetry_repo: FileStateRepository<Vec<TelemetryInterfaceConfig>> =
            FileStateRepository::new(&store_directory, TELEMETRY_PATH);
        if telemetry_repo.exists().await {
            let saved_config: Vec<TelemetryInterfaceConfig> = telemetry_repo.read().await.unwrap();
            for c in saved_config {
                if let Some(rwlock_default_task) = telemetry_task_configs.get_mut(&c.interface_name)
                {
                    rwlock_default_task.override_enabled = c.enabled;
                    rwlock_default_task.override_period = c.period;
                } else {
                    telemetry_task_configs.insert(
                        c.interface_name.clone(),
                        TelemetryTaskConfig {
                            default_enabled: None,
                            default_period: None,
                            override_enabled: c.enabled,
                            override_period: c.period,
                        },
                    );
                };
            }
        }

        Telemetry {
            telemetry_task_configs: Arc::new(RwLock::new(telemetry_task_configs)),
            kill_switches: HashMap::new(),
            communication_channel,
            store_directory,
        }
    }

    pub async fn run_telemetry(&mut self) {
        for interface_name in self.telemetry_task_configs.clone().read().await.keys() {
            self.schedule_task(interface_name.clone()).await;
        }
    }

    async fn schedule_task(&mut self, interface_name: String) {
        let telemetry_task_configs_clone = self.telemetry_task_configs.clone();
        let telemetry_task_configs = telemetry_task_configs_clone.read().await;
        let telemetry_task_config = telemetry_task_configs.get(&interface_name.clone()).unwrap();

        let period = telemetry_task_config
            .override_period
            .unwrap_or_else(|| telemetry_task_config.default_period.unwrap_or(0));

        let enabled = telemetry_task_config
            .override_enabled
            .unwrap_or_else(|| telemetry_task_config.default_enabled.unwrap_or(false));

        if let Some(kill_switch) = self.kill_switches.get(&interface_name.clone()) {
            let _ = kill_switch.send(());
        }

        let comm = self.communication_channel.clone();

        if period > 0 && enabled {
            let (tx, rx) = channel(1);
            spawn(Telemetry::start_task(
                rx,
                interface_name.clone(),
                period,
                comm,
            ));

            self.kill_switches.insert(interface_name, tx);
        }
    }

    async fn start_task(
        mut kill_switch: Receiver<()>,
        interface_name: String,
        period: u64,
        communication_channel: MpscSender<TelemetryMessage>,
    ) {
        tokio::select! {
            _output = Telemetry::data_send_loop(interface_name, period, communication_channel) => {debug!("data_send_loop ended")},
            _ = kill_switch.recv() => {debug!("Kill switch triggered")},
        }
    }

    async fn data_send_loop(
        interface_name: String,
        period: u64,
        communication_channel: MpscSender<TelemetryMessage>,
    ) {
        let mut interval = interval(Duration::from_secs(period));
        loop {
            interval.tick().await;

            // TODO: the error should be bubbled up
            if let Err(err) = send_data(&communication_channel, &interface_name).await {
                error!("couldn't send telemetry data: {:#?}", err)
            }
        }
    }

    async fn set_enabled(&self, interface_name: &str, enabled: bool) {
        debug!("set {interface_name} to enabled {enabled}");

        self.telemetry_task_configs
            .clone()
            .write()
            .await
            .entry(interface_name.to_string())
            .or_insert_with(Default::default)
            .override_enabled = Some(enabled);
    }

    async fn unset_enabled(&self, interface_name: &str) {
        debug!("unset {interface_name} enabled");

        if let Some(telemetry_task_config) = self
            .telemetry_task_configs
            .clone()
            .write()
            .await
            .get_mut(interface_name)
        {
            telemetry_task_config.override_enabled = None;
        }
    }

    async fn set_period(&self, interface_name: &str, period: u64) {
        debug!("set {interface_name} to period {period}");
        self.telemetry_task_configs
            .clone()
            .write()
            .await
            .entry(interface_name.to_string())
            .or_insert_with(Default::default)
            .override_period = Some(period);
    }

    async fn unset_period(&self, interface_name: &str) {
        debug!("unset {interface_name} period");

        if let Some(telemetry_task_config) = self
            .telemetry_task_configs
            .clone()
            .write()
            .await
            .get_mut(interface_name)
        {
            telemetry_task_config.override_period = None;
        }
    }

    pub async fn telemetry_config_event(
        &mut self,
        interface_name: &str,
        endpoint: &str,
        data: Option<&AstarteType>,
    ) {
        match (endpoint, data) {
            ("enable", Some(AstarteType::Boolean(enabled))) => {
                self.set_enabled(interface_name, *enabled).await;
            }

            ("enable", None) => {
                self.unset_enabled(interface_name).await;
            }

            ("periodSeconds", Some(AstarteType::LongInteger(period))) => {
                self.set_period(interface_name, *period as u64).await;
            }

            ("periodSeconds", Some(AstarteType::Integer(period))) => {
                self.set_period(interface_name, *period as u64).await;
            }

            ("periodSeconds", None) => {
                self.unset_period(interface_name).await;
            }

            _ => {
                warn!("Received malformed data from io.edgehog.devicemanager.config.Telemetry: {endpoint} {data:?}");
            }
        }

        self.schedule_task(interface_name.to_string()).await;
        self.save_telemetry_config().await;
    }

    async fn save_telemetry_config(&self) {
        let mut telemetry_config: Vec<TelemetryInterfaceConfig> = Vec::new();
        for (interface_name, telemetry_task_config) in
            &*self.telemetry_task_configs.clone().read().await
        {
            let interface_config = TelemetryInterfaceConfig {
                interface_name: interface_name.to_string(),
                enabled: telemetry_task_config.override_enabled,
                period: telemetry_task_config.override_period,
            };

            telemetry_config.push(interface_config);
        }

        let telemetry_repo = FileStateRepository::new(&self.store_directory, TELEMETRY_PATH);
        if let Err(err) = telemetry_repo.write(&telemetry_config).await {
            error!("failed to write telemetry: {err}");
        }
    }
}

async fn send_data(
    communication_channel: &MpscSender<TelemetryMessage>,
    interface_name: &str,
) -> Result<(), DeviceManagerError> {
    debug!("sending {interface_name}");

    match interface_name {
        "io.edgehog.devicemanager.SystemStatus" => {
            let sysstatus = system_status::get_system_status()?;
            let _ = communication_channel
                .send(TelemetryMessage {
                    path: "".to_string(),
                    payload: TelemetryPayload::SystemStatus(sysstatus),
                })
                .await;
        }
        "io.edgehog.devicemanager.StorageUsage" => {
            let storage_usage = storage_usage::get_storage_usage();
            for (path, payload) in storage_usage {
                let _ = communication_channel
                    .send(TelemetryMessage {
                        path,
                        payload: TelemetryPayload::StorageUsage(payload),
                    })
                    .await;
            }
        }
        "io.edgehog.devicemanager.BatteryStatus" => {
            let battery_status = battery_status::get_battery_status().await?;
            for (path, payload) in battery_status {
                let _ = communication_channel
                    .send(TelemetryMessage {
                        path,
                        payload: TelemetryPayload::BatteryStatus(payload),
                    })
                    .await;
            }
        }
        interface => {
            warn!("unimplemented telemetry interface {}", interface)
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::repository::file_state_repository::FileStateRepository;
    use crate::repository::StateRepository;
    use crate::telemetry::{send_data, Telemetry, TelemetryInterfaceConfig};

    use astarte_device_sdk::types::AstarteType;
    use tempdir::TempDir;

    const TELEMETRY_PATH: &str = "telemetry.json";

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new("edgehog").unwrap();
        let path = dir.path().to_owned();

        (dir, path)
    }

    #[tokio::test]
    async fn telemetry_default_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: Some(true),
            period: Some(10),
        });

        let (_dir, t_dir) = temp_dir();

        let (tx, _) = tokio::sync::mpsc::channel(32);
        let tel = Telemetry::from_default_config(Some(config), tx, t_dir).await;
        let telemetry_config = tel.telemetry_task_configs.clone();
        let interface_configs = telemetry_config.read().await;
        let system_status_config = interface_configs.get(interface_name).unwrap();

        assert!(system_status_config.default_enabled.unwrap());
        assert_eq!(system_status_config.default_period.unwrap(), 10);
    }

    #[tokio::test]
    async fn telemetry_set_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: Some(true),
            period: Some(10),
        });

        let (_dir, t_dir) = temp_dir();

        let (tx, _) = tokio::sync::mpsc::channel(32);
        let mut tel = Telemetry::from_default_config(Some(config), tx, t_dir.clone()).await;

        tel.telemetry_config_event(interface_name, "enable", Some(&AstarteType::Boolean(false)))
            .await;
        tel.telemetry_config_event(
            interface_name,
            "periodSeconds",
            Some(&AstarteType::LongInteger(30)),
        )
        .await;

        let telemetry_config = tel.telemetry_task_configs.clone();
        let config = telemetry_config.read().await;

        assert!(!config
            .get(interface_name)
            .unwrap()
            .override_enabled
            .unwrap());
        assert_eq!(
            config.get(interface_name).unwrap().override_period.unwrap(),
            30
        );

        let telemetry_repo = FileStateRepository::new(&t_dir, TELEMETRY_PATH);
        let saved_config: Vec<TelemetryInterfaceConfig> = telemetry_repo.read().await.unwrap();

        assert_eq!(saved_config.len(), 1);

        let system_status_config = saved_config.first().unwrap();
        assert_eq!(system_status_config.enabled, Some(false));
        assert_eq!(system_status_config.period, Some(30));
    }

    #[tokio::test]
    async fn telemetry_unset_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: Some(true),
            period: Some(10),
        });

        let (_dir, t_dir) = temp_dir();

        let (tx, _) = tokio::sync::mpsc::channel(32);
        let mut tel = Telemetry::from_default_config(Some(config), tx, t_dir.clone()).await;

        tel.telemetry_config_event(interface_name, "enable", None)
            .await;
        tel.telemetry_config_event(interface_name, "periodSeconds", None)
            .await;

        let telemetry_config = tel.telemetry_task_configs.clone();
        let config = telemetry_config.read().await;

        assert!(config
            .get(interface_name)
            .unwrap()
            .override_enabled
            .is_none());
        assert!(config
            .get(interface_name)
            .unwrap()
            .override_period
            .is_none());

        let telemetry_repo = FileStateRepository::new(&t_dir, TELEMETRY_PATH);
        let saved_config: Vec<TelemetryInterfaceConfig> = telemetry_repo.read().await.unwrap();

        assert_eq!(saved_config.len(), 1);

        let system_status_config = saved_config.first().unwrap();
        assert!(system_status_config.enabled.is_none());
        assert!(system_status_config.period.is_none());
    }

    #[tokio::test]
    async fn telemetry_message_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: Some(true),
            period: Some(10),
        });

        let (_dir, t_dir) = temp_dir();

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let mut tel = Telemetry::from_default_config(Some(config), tx, t_dir).await;
        tel.telemetry_config_event(interface_name, "enable", Some(&AstarteType::Boolean(true)))
            .await;
        tel.telemetry_config_event(
            interface_name,
            "periodSeconds",
            Some(&AstarteType::LongInteger(10)),
        )
        .await;

        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn from_default_config_null_test() {
        let (_dir, t_dir) = temp_dir();

        let (tx, _) = tokio::sync::mpsc::channel(32);
        let tel = Telemetry::from_default_config(None, tx, t_dir).await;
        assert!(tel.telemetry_task_configs.clone().read().await.is_empty());
    }

    #[tokio::test]
    async fn send_data_test() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let interfaces = [
            "io.edgehog.devicemanager.SystemStatus",
            "io.edgehog.devicemanager.StorageUsage",
            "io.edgehog.devicemanager.BatteryStatus",
        ];

        for interface in interfaces {
            let res = send_data(&tx, interface).await;

            assert!(
                res.is_ok(),
                "failed to send '{}' data for interface: {}",
                interface,
                res.unwrap_err()
            );

            assert!(rx.recv().await.is_some());
        }
    }
}
