// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::{borrow::Cow, collections::HashMap, ops::Deref, path::PathBuf, str::FromStr};

use async_trait::async_trait;
#[cfg(all(feature = "zbus", target_os = "linux"))]
use cellular_properties::CellularConnection;
use event::{TelemetryConfig, TelemetryEvent};
use serde::{Deserialize, Serialize};
use system_info::SystemInfo;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{
    controller::actor::Actor,
    data::Publisher,
    repository::{file_state_repository::FileStateRepository, StateRepository},
};

use self::{
    hardware_info::HardwareInfo,
    os_release::OsRelease,
    runtime_info::RUNTIME_INFO,
    sender::{Task, TelemetryInterface},
    storage_usage::StorageUsage,
};

#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod battery_status;
#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod cellular_properties;
pub mod event;
pub mod hardware_info;
#[cfg(feature = "udev")]
pub(crate) mod net_interfaces;
pub mod os_release;
pub mod runtime_info;
pub mod sender;
pub(crate) mod storage_usage;
pub(crate) mod system_info;
pub(crate) mod system_status;
#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod upower;
#[cfg(feature = "wifiscanner")]
pub(crate) mod wifi_scan;

const TELEMETRY_PATH: &str = "telemetry.json";

const DEFAULT_PERIOD: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryInterfaceConfig<'a> {
    pub interface_name: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<u64>,
}

impl TelemetryInterfaceConfig<'_> {
    fn period_duration(&self) -> Option<Duration> {
        self.period.map(Duration::from_secs)
    }
}

/// Configuration for the tasks.
#[derive(Debug, Clone, Copy)]
pub struct TaskConfig {
    pub enabled: Overridable<bool>,
    pub period: Overridable<Duration>,
}

impl TaskConfig {
    /// Creates a tasks configuration from the one from the file.
    fn from_config(config: &TelemetryInterfaceConfig) -> Self {
        Self {
            enabled: Overridable::new(config.enabled.unwrap_or_default()),
            period: Overridable::new(config.period_duration().unwrap_or(DEFAULT_PERIOD)),
        }
    }

    /// Creates a task config from the override, with default defaults
    fn from_override(over: &TelemetryInterfaceConfig) -> Option<Self> {
        if over.enabled.is_none() && over.period.is_none() {
            return None;
        }

        let enabled = match over.enabled {
            Some(enabled) => Overridable::with_override(false, enabled),
            None => Overridable::new(false),
        };
        let period = match over.period_duration() {
            Some(period) => Overridable::with_override(DEFAULT_PERIOD, period),
            None => Overridable::new(DEFAULT_PERIOD),
        };

        Some(Self { enabled, period })
    }
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            enabled: Overridable::new(false),
            period: Overridable::new(DEFAULT_PERIOD),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Overridable<T> {
    default: T,
    value: Option<T>,
}

impl<T> Overridable<T> {
    #[must_use]
    fn new(default: T) -> Self {
        Self {
            default,
            value: None,
        }
    }

    #[must_use]
    fn with_override(default: T, value: T) -> Self {
        Self {
            default,
            value: Some(value),
        }
    }

    #[must_use]
    fn get(&self) -> &T {
        self.value.as_ref().unwrap_or(&self.default)
    }

    #[must_use]
    fn get_override(&self) -> Option<&T> {
        self.value.as_ref()
    }

    fn set(&mut self, value: T) {
        self.value.replace(value);
    }

    fn unset(&mut self) {
        self.value.take();
    }

    /// Returns `true` if the overridable has a custom value.
    #[must_use]
    fn is_overwritten(&self) -> bool {
        self.value.is_some()
    }
}

impl<T> Default for Overridable<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Deref for Overridable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

#[derive(Debug)]
pub struct Telemetry<T> {
    client: T,
    configs: HashMap<TelemetryInterface, TaskConfig>,
    cancellation: CancellationToken,
    tasks: HashMap<TelemetryInterface, CancellationToken>,
    file_state: FileStateRepository<Vec<TelemetryInterfaceConfig<'static>>>,
}

impl<T> Telemetry<T> {
    pub async fn from_config<'a>(
        client: T,
        configs: &[TelemetryInterfaceConfig<'a>],
        store_directory: PathBuf,
    ) -> Self {
        let configs = configs
            .iter()
            .filter_map(|cfg| {
                let interface = match TelemetryInterface::from_str(&cfg.interface_name) {
                    Ok(interface) => interface,
                    Err(err) => {
                        error!("{err}");

                        return None;
                    }
                };

                Some((interface, TaskConfig::from_config(cfg)))
            })
            .collect();

        let mut telemetry = Telemetry {
            client,
            configs,
            cancellation: CancellationToken::new(),
            tasks: HashMap::new(),
            file_state: FileStateRepository::new(&store_directory, TELEMETRY_PATH),
        };

        telemetry.read_filestate().await;

        telemetry
    }

    async fn read_filestate(&mut self) {
        if !self.file_state.exists().await {
            return;
        }

        let saved_configs = match self.file_state.read().await {
            Ok(cfgs) => cfgs,
            Err(err) => {
                // Don't error here since the file is corrupted, but it will be overwritten
                error!(
                    "couldn't read the saved telemetry configs: {}",
                    stable_eyre::Report::new(err)
                );

                return;
            }
        };

        for saved_cfg in saved_configs {
            let interface = match TelemetryInterface::from_str(&saved_cfg.interface_name) {
                Ok(interface) => interface,
                Err(err) => {
                    error!("{err}");

                    continue;
                }
            };

            let entry = self.configs.entry(interface).and_modify(|cfg| {
                if let Some(enabled) = saved_cfg.enabled {
                    cfg.enabled.set(enabled);
                }
                if let Some(period) = saved_cfg.period_duration() {
                    cfg.period.set(period);
                }
            });

            if let Some(cfg) = TaskConfig::from_override(&saved_cfg) {
                entry.or_insert(cfg);
            }
        }
    }

    async fn initial_telemetry(&self)
    where
        T: Publisher,
    {
        #[cfg(feature = "systemd")]
        crate::systemd_wrapper::systemd_notify_status("Sending initial telemetry");

        if let Some(os_release) = OsRelease::read().await {
            os_release.send(&self.client).await;
        }

        HardwareInfo::read().await.send(&self.client).await;

        RUNTIME_INFO.send(&self.client).await;

        #[cfg(feature = "udev")]
        net_interfaces::send_network_interface_properties(&self.client).await;

        SystemInfo::read().send(&self.client).await;

        StorageUsage::read().send(&self.client).await;

        #[cfg(feature = "wifiscanner")]
        wifi_scan::send_wifi_scan(&self.client).await;

        #[cfg(all(feature = "zbus", target_os = "linux"))]
        CellularConnection::read().await.send(&self.client).await;
    }

    pub fn run_telemetry(&mut self)
    where
        T: Publisher + Clone + Send + Sync + 'static,
    {
        for (t_itf, task_config) in &self.configs {
            Self::spawn_task(
                &mut self.tasks,
                &self.client,
                &self.cancellation,
                *t_itf,
                *task_config,
            );
        }
    }

    // Cursed arguments to borrow tasks mutably while iterating above
    fn spawn_task(
        tasks: &mut HashMap<TelemetryInterface, CancellationToken>,
        client: &T,
        cancellation: &CancellationToken,
        t_itf: TelemetryInterface,
        task_config: TaskConfig,
    ) where
        T: Publisher + Clone + Sync + Send + 'static,
    {
        if !task_config.enabled.get() {
            debug!("task {} disabled", t_itf);

            if let Some(cancel) = tasks.remove(&t_itf) {
                cancel.cancel();
            }

            return;
        }

        let period = task_config.period.get();
        if period.is_zero() {
            debug!("period is 0 for task {}", t_itf);

            if let Some(cancel) = tasks.remove(&t_itf) {
                cancel.cancel();
            }

            return;
        };

        if let Some(cancel) = tasks.remove(&t_itf) {
            debug!("stopping previour task");

            cancel.cancel();
        }

        let cancel = cancellation.child_token();
        let task = Task::new(client.clone(), t_itf, cancel.clone(), *period);

        tokio::spawn(async move { task.run().await });

        tasks.insert(t_itf, cancel);
    }

    async fn save_telemetry_config(&self) {
        let telemetry_config = self
            .configs
            .iter()
            .filter_map(|(interface, cfg)| {
                if cfg.enabled.is_overwritten() || cfg.period.is_overwritten() {
                    Some(TelemetryInterfaceConfig {
                        interface_name: Cow::Borrowed(interface.as_interface()),
                        enabled: cfg.enabled.get_override().copied(),
                        period: cfg.period.get_override().map(Duration::as_secs),
                    })
                } else {
                    None
                }
            })
            .collect();

        if let Err(err) = self.file_state.write(&telemetry_config).await {
            error!(
                "failed to write telemetry: {}",
                stable_eyre::Report::new(err)
            );
        }
    }
}

#[async_trait]
impl<T> Actor for Telemetry<T>
where
    T: Publisher + Clone + Send + Sync + 'static,
{
    type Msg = TelemetryEvent;

    fn task() -> &'static str {
        "telemetry"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        self.initial_telemetry().await;

        self.run_telemetry();

        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        let interface = match TelemetryInterface::from_str(&msg.interface) {
            Ok(itf) => itf,
            Err(err) => {
                error!("{err}");

                return Ok(());
            }
        };

        let config = self.configs.entry(interface).or_default();

        match msg.config {
            TelemetryConfig::Enable(Some(enabled)) => {
                config.enabled.set(enabled);
            }
            TelemetryConfig::Enable(None) => {
                config.enabled.unset();
            }
            TelemetryConfig::Period(Some(period)) => {
                config.period.set(period.0);
            }
            TelemetryConfig::Period(None) => {
                config.period.unset();
            }
        };

        // This function will check if we actually need to start the task
        Self::spawn_task(
            &mut self.tasks,
            &self.client,
            &self.cancellation,
            interface,
            *config,
        );
        self.save_telemetry_config().await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::data::tests::MockPubSub;

    use event::TelemetryPeriod;
    use mockall::Sequence;
    use storage_usage::DiskUsage;
    use tempdir::TempDir;

    const TELEMETRY_PATH: &str = "telemetry.json";

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new("edgehog-telemetry").unwrap();
        let path = dir.path().to_owned();

        (dir, path)
    }

    fn mock_telemetry(client: MockPubSub) -> (Telemetry<MockPubSub>, TempDir) {
        let (dir, path) = temp_dir();

        (
            Telemetry {
                client,
                configs: HashMap::new(),
                cancellation: CancellationToken::new(),
                tasks: HashMap::new(),
                file_state: FileStateRepository::new(&path, TELEMETRY_PATH),
            },
            dir,
        )
    }

    #[tokio::test]
    async fn telemetry_default_test() {
        let interface = "io.edgehog.devicemanager.SystemStatus";
        let configs = vec![TelemetryInterfaceConfig {
            interface_name: std::borrow::Cow::Borrowed(interface),
            enabled: Some(true),
            period: Some(10),
        }];

        let (_dir, t_dir) = temp_dir();

        let client = MockPubSub::new();

        let tel = Telemetry::from_config(client, &configs, t_dir).await;

        let system_status_config = tel.configs.get(&TelemetryInterface::SystemStatus).unwrap();

        assert!(system_status_config.enabled.get());
        assert_eq!(*system_status_config.period.get(), Duration::from_secs(10));
    }

    #[tokio::test]
    async fn telemetry_set_test() {
        let interface = "io.edgehog.devicemanager.SystemStatus";
        let configs = vec![TelemetryInterfaceConfig {
            interface_name: interface.into(),
            enabled: Some(true),
            period: Some(10),
        }];

        let (_dir, t_dir) = temp_dir();

        let client = MockPubSub::new();

        let mut tel = Telemetry::from_config(client, &configs, t_dir.clone()).await;

        let events = [
            TelemetryEvent {
                interface: interface.to_string(),
                config: TelemetryConfig::Enable(Some(false)),
            },
            TelemetryEvent {
                interface: interface.to_string(),
                config: TelemetryConfig::Period(Some(TelemetryPeriod(Duration::from_secs(30)))),
            },
        ];

        for e in events {
            tel.handle(e).await.unwrap();
        }

        let config = tel.configs.get(&TelemetryInterface::SystemStatus).unwrap();

        assert!(config.enabled.is_overwritten());
        assert!(!config.enabled.get());
        assert!(config.period.is_overwritten());
        assert_eq!(*config.period.get(), Duration::from_secs(30));

        let telemetry_repo = FileStateRepository::new(&t_dir, TELEMETRY_PATH);
        let saved_config: Vec<TelemetryInterfaceConfig> = telemetry_repo.read().await.unwrap();

        assert_eq!(saved_config.len(), 1);

        let system_status_config = saved_config.first().unwrap();
        assert_eq!(system_status_config.enabled, Some(false));
        assert_eq!(system_status_config.period, Some(30));
    }

    #[tokio::test]
    async fn telemetry_unset_test() {
        let interface = "io.edgehog.devicemanager.SystemStatus";
        let configs = vec![TelemetryInterfaceConfig {
            interface_name: interface.into(),
            enabled: Some(true),
            period: Some(10),
        }];

        let (_dir, t_dir) = temp_dir();

        let mut client = MockPubSub::new();
        let mut seq = Sequence::new();

        client
            .expect_clone()
            .times(2)
            .in_sequence(&mut seq)
            .returning(MockPubSub::new);

        let mut tel = Telemetry::from_config(client, &configs, t_dir.clone()).await;

        let events = [
            TelemetryEvent {
                interface: interface.to_string(),
                config: TelemetryConfig::Enable(None),
            },
            TelemetryEvent {
                interface: interface.to_string(),
                config: TelemetryConfig::Period(None),
            },
        ];

        for e in events {
            tel.handle(e).await.unwrap();
        }

        let config = tel.configs.get(&TelemetryInterface::SystemStatus).unwrap();

        assert!(!config.enabled.is_overwritten());
        assert!(config.enabled.get());
        assert!(!config.period.is_overwritten());
        assert_eq!(*config.period.get(), Duration::from_secs(10));

        let telemetry_repo = FileStateRepository::new(&t_dir, TELEMETRY_PATH);
        let saved_config: Vec<TelemetryInterfaceConfig> = telemetry_repo.read().await.unwrap();

        assert!(saved_config.is_empty());
    }

    #[tokio::test]
    async fn send_initial_telemetry_success() {
        let mut client = MockPubSub::new();

        client
            .expect_send()
            .times(1)
            .withf(|interface, _, _| interface == "io.edgehog.devicemanager.OSInfo")
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(..)
            .withf(move |interface_name, _, _| {
                interface_name == "io.edgehog.devicemanager.HardwareInfo"
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(..)
            .withf(|interface_name, _, _| interface_name == "io.edgehog.devicemanager.RuntimeInfo")
            .returning(|_, _, _| Ok(()));
        client
            .expect_send()
            .once()
            .withf(|interface_name, _, _| interface_name == "io.edgehog.devicemanager.OSInfo")
            .returning(|_, _, _| Ok(()));

        client
            .expect_send_object::<DiskUsage>()
            .times(..)
            .withf(|interface, _, _| interface == "io.edgehog.devicemanager.StorageUsage")
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(..)
            .withf(|interface_name, _, _| {
                interface_name == "io.edgehog.devicemanager.NetworkInterfaceProperties"
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .withf(|interface_name, _, _| interface_name == "io.edgehog.devicemanager.SystemInfo")
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .withf(|interface_name, _, _| interface_name == "io.edgehog.devicemanager.BaseImage")
            .returning(|_, _, _| Ok(()));

        let (telemetry, _dir) = mock_telemetry(client);

        telemetry.initial_telemetry().await;
    }
}
