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

use astarte_sdk::{types::AstarteType, AstarteSdk};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Notify;

pub(crate) mod hardware_info;
pub(crate) mod net_if_properties;
pub(crate) mod os_info;
pub(crate) mod runtime_info;
pub(crate) mod storage_usage;
pub(crate) mod system_info;
pub(crate) mod system_status;
pub(crate) mod wifi_scan;

const TELEMETRY_PATH: &str = "telemetry.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryInterfaceConfig {
    pub interface_name: String,
    pub enabled: bool,
    pub period: u64,
}
pub struct Telemetry {
    default_config: Arc<HashMap<String, TelemetryInterfaceConfig>>,
    override_enabled: Arc<tokio::sync::RwLock<std::collections::HashMap<String, bool>>>,
    override_period: Arc<tokio::sync::RwLock<std::collections::HashMap<String, u64>>>,
    notify: std::collections::HashMap<String, Arc<Notify>>,
}

impl Telemetry {
    pub async fn from_default_config(cfg: Vec<TelemetryInterfaceConfig>) -> Self {
        let mut default_config = HashMap::new();
        let mut notify = HashMap::new();

        for c in cfg {
            let name = c.interface_name.clone();
            default_config.insert(name.clone(), c);
            let not = Arc::new(Notify::new());
            not.notify_one();
            notify.insert(name.clone(), not);
        }

        let ret = Telemetry {
            default_config: Arc::new(default_config),
            override_enabled: Default::default(),
            override_period: Default::default(),
            notify,
        };

        // Serde load configs from disk
        if let Ok(file) = File::open(TELEMETRY_PATH) {
            let saved_config: HashMap<String, TelemetryInterfaceConfig> =
                serde_json::from_reader(BufReader::new(file)).unwrap();
            let self_override_period = ret.override_period.clone();
            let self_override_enabled = ret.override_enabled.clone();
            for conf in saved_config.values() {
                let name = conf.interface_name.clone();
                *self_override_enabled
                    .write()
                    .await
                    .entry(name.to_string())
                    .or_insert(conf.enabled) = conf.enabled;
                *self_override_period
                    .write()
                    .await
                    .entry(name.to_string())
                    .or_insert(conf.period) = conf.period;
            }
        };

        ret
    }

    pub async fn run_telemetry(&self, sdk: AstarteSdk) {
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        for (interface_name, interface_cfg) in (*self.default_config).clone() {
            let self_override_period = self.override_period.clone();
            let self_override_enabled = self.override_enabled.clone();
            let self_notify = self.notify.clone();

            let txcl = tx.clone();

            // task runs for every interface
            tokio::task::spawn(async move {
                loop {
                    self_notify.get(&interface_name).unwrap().notified().await;

                    let enabled = *self_override_enabled
                        .read()
                        .await
                        .get(&interface_name)
                        .unwrap_or(&interface_cfg.enabled);

                    if enabled {
                        self_notify.get(&interface_name).unwrap().notify_one();
                    }

                    txcl.send(interface_name.clone()).await.unwrap();

                    let period = *self_override_period
                        .read()
                        .await
                        .get(&interface_name)
                        .unwrap_or(&interface_cfg.period);

                    tokio::time::sleep(std::time::Duration::from_secs(period)).await;
                }
            });
        }

        while let Some(interface_name) = rx.recv().await {
            send_data(&sdk, interface_name).await;
        }
    }

    pub async fn set_enabled(&self, interface_name: &str, enabled: bool) {
        debug!("set {interface_name} to enabled {enabled}");

        *self
            .override_enabled
            .write()
            .await
            .entry(interface_name.to_string())
            .or_insert(enabled) = enabled;

        if enabled {
            self.notify.get(interface_name).unwrap().notify_one();
        }

        self.save_telemetry_config().await;
    }

    pub async fn unset_enabled(&self, interface_name: &str) {
        debug!("unset {interface_name} enabled");

        self.override_enabled.write().await.remove(interface_name);

        self.save_telemetry_config().await;
    }

    pub async fn set_period(&self, interface_name: &str, period: u64) {
        debug!("set {interface_name} to period {period}");

        *self
            .override_period
            .write()
            .await
            .entry(interface_name.to_string())
            .or_insert(period) = period;

        self.save_telemetry_config().await;
    }

    pub async fn unset_period(&self, interface_name: &str) {
        debug!("unset {interface_name} period");

        self.override_period.write().await.remove(interface_name);

        self.save_telemetry_config().await;
    }

    pub async fn telemetry_config_event(
        &self,
        interface_name: &str,
        endpoint: &str,
        data: &AstarteType,
    ) {
        match (endpoint, data) {
            ("enable", AstarteType::Boolean(enabled)) => {
                self.set_enabled(interface_name, *enabled).await;
            }

            ("enable", AstarteType::Unset) => {
                self.unset_enabled(interface_name).await;
            }

            ("periodSeconds", AstarteType::LongInteger(period)) => {
                self.set_period(interface_name, *period as u64).await;
            }

            ("periodSeconds", AstarteType::Integer(period)) => {
                self.set_period(interface_name, *period as u64).await;
            }

            ("periodSeconds", AstarteType::Unset) => {
                self.unset_period(interface_name).await;
            }

            _ => {
                warn!("Received malformed data from io.edgehog.devicemanager.config.Telemetry: {endpoint} {data:?}");
            }
        }
    }

    async fn save_telemetry_config(&self) {
        let mut telemetry_config: HashMap<String, TelemetryInterfaceConfig> = HashMap::new();
        for path in self.default_config.clone().keys() {
            let interface_config = TelemetryInterfaceConfig {
                interface_name: path.to_string(),
                enabled: if let Some(enabled) = self.override_enabled.read().await.get(path) {
                    enabled.clone()
                } else {
                    self.default_config.clone().get(path).unwrap().enabled
                },
                period: if let Some(period) = self.override_period.read().await.get(path) {
                    period.clone()
                } else {
                    self.default_config.clone().get(path).unwrap().period
                },
            };
            telemetry_config.insert(path.to_string(), interface_config);
        }
        let file = File::create(TELEMETRY_PATH).unwrap();
        serde_json::to_writer(BufWriter::new(file), &telemetry_config).unwrap();
    }
}

async fn send_data(sdk: &AstarteSdk, interface_name: String) {
    debug!("sending {interface_name}");

    if interface_name.as_str() == "io.edgehog.devicemanager.SystemStatus" {
        let sysstatus = system_status::get_system_status().unwrap();
        sdk.send_object(
            "io.edgehog.devicemanager.SystemStatus",
            "/systemStatus",
            sysstatus,
        )
        .await
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use crate::telemetry::{Telemetry, TelemetryInterfaceConfig};

    #[tokio::test]
    async fn telemetry_default_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: true,
            period: 10,
        });
        let tel = Telemetry::from_default_config(config).await;
        let telemetry_default = &*tel.default_config.clone();
        let default_interface_config = telemetry_default.get(interface_name).unwrap();

        assert_eq!(
            default_interface_config.interface_name,
            interface_name.to_string()
        );
        assert!(default_interface_config.enabled);
        assert_eq!(default_interface_config.period, 10);
    }

    #[tokio::test]
    async fn telemetry_set_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: true,
            period: 10,
        });

        let tel = Telemetry::from_default_config(config).await;

        tel.set_enabled(&interface_name, false).await;
        tel.set_period(&interface_name, 30).await;

        assert!(!(*tel.override_enabled.clone())
            .read()
            .await
            .get(interface_name)
            .unwrap());
        assert_eq!(
            (*tel.override_period.clone())
                .read()
                .await
                .get(interface_name)
                .unwrap(),
            &30
        );
    }

    #[tokio::test]
    async fn telemetry_unset_test() {
        let mut config = Vec::new();
        let interface_name = "io.edgehog.devicemanager.SystemStatus";
        config.push(TelemetryInterfaceConfig {
            interface_name: interface_name.to_string(),
            enabled: true,
            period: 10,
        });

        let tel = Telemetry::from_default_config(config).await;

        tel.unset_enabled(&interface_name).await;
        tel.unset_period(&interface_name).await;

        assert!((*tel.override_enabled.clone())
            .read()
            .await
            .get(interface_name)
            .is_none());
        assert!((*tel.override_period.clone())
            .read()
            .await
            .get(interface_name)
            .is_none());
    }
}
