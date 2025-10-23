// This file is part of Edgehog.
//
// Copyright 2022 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;
use std::{borrow::Cow, collections::HashMap, ops::Deref, path::PathBuf};

use async_trait::async_trait;
use event::{TelemetryConfig, TelemetryEvent};
use serde::{Deserialize, Serialize};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::Client;

use crate::{
    controller::actor::Actor,
    repository::{file_state_repository::FileStateRepository, StateRepository},
};

use self::sender::Task;
use self::stats::TelemetryInterface;

pub mod event;
mod sender;
mod stats;
pub mod status;

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
pub struct Telemetry<C> {
    client: C,
    configs: HashMap<TelemetryInterface, TaskConfig>,
    tasks: TelemetryTasks,
    file_state: FileStateRepository<Vec<TelemetryInterfaceConfig<'static>>>,
    #[cfg(feature = "containers")]
    containers: std::sync::Arc<tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>>,
}

impl<C> Telemetry<C> {
    pub async fn from_config(
        client: C,
        configs: &[TelemetryInterfaceConfig<'_>],
        store_directory: PathBuf,
        #[cfg(feature = "containers")] containers: std::sync::Arc<
            tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>,
        >,
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
            tasks: TelemetryTasks::new(),
            file_state: FileStateRepository::new(&store_directory, TELEMETRY_PATH),
            #[cfg(feature = "containers")]
            containers,
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

    async fn initial_telemetry(&mut self)
    where
        C: Client + Send + Sync + 'static,
    {
        self::status::initial_telemetry(&mut self.client).await;
        self::stats::initial_telemetry(&mut self.client).await;
    }

    pub fn run_telemetry(&mut self)
    where
        C: Client + Send + Sync + 'static,
    {
        for (interface, config) in &self.configs {
            self.tasks.spawn_task(
                &self.client,
                *interface,
                *config,
                #[cfg(feature = "containers")]
                &self.containers,
            );
        }
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
impl<C> Actor for Telemetry<C>
where
    C: Client + Send + Sync + 'static,
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
                error!(
                    error = format!("{:#}", stable_eyre::Report::new(err)),
                    "couldn't parse telemetry interface"
                );

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
        self.tasks.spawn_task(
            &self.client,
            interface,
            *config,
            #[cfg(feature = "containers")]
            &self.containers,
        );

        self.save_telemetry_config().await;

        Ok(())
    }
}

/// Handle spawning and cancellation for the telemetry tasks
#[derive(Debug)]
struct TelemetryTasks {
    cancellation: CancellationToken,
    tasks: HashMap<TelemetryInterface, CancellationToken>,
}

impl TelemetryTasks {
    fn new() -> Self {
        Self {
            cancellation: CancellationToken::new(),
            tasks: HashMap::new(),
        }
    }

    fn spawn_task<C>(
        &mut self,
        client: &C,
        t_itf: TelemetryInterface,
        task_config: TaskConfig,
        #[cfg(feature = "containers")] containers: &std::sync::Arc<
            tokio::sync::OnceCell<edgehog_containers::local::ContainerHandle>,
        >,
    ) where
        C: Client + Sync + Send + 'static,
    {
        if !task_config.enabled.get() {
            debug!("task {} disabled", t_itf);

            if let Some(cancel) = self.tasks.remove(&t_itf) {
                cancel.cancel();
            }

            return;
        }

        let period = task_config.period.get();
        if period.is_zero() {
            debug!("period is 0 for task {}", t_itf);

            if let Some(cancel) = self.tasks.remove(&t_itf) {
                cancel.cancel();
            }

            return;
        };

        if let Some(cancel) = self.tasks.remove(&t_itf) {
            debug!("stopping previous task");

            cancel.cancel();
        }

        let cancel = self.cancellation.child_token();

        Task::spawn(
            client.clone(),
            cancel.clone(),
            t_itf,
            *period,
            #[cfg(feature = "containers")]
            containers,
        );

        self.tasks.insert(t_itf, cancel);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use event::TelemetryPeriod;
    use mockall::{predicate, Sequence};
    use tempdir::TempDir;

    use super::status::runtime_info::tests::mock_runtime_info_telemetry;

    const TELEMETRY_PATH: &str = "telemetry.json";

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new("edgehog-telemetry").unwrap();
        let path = dir.path().to_owned();

        (dir, path)
    }

    fn mock_telemetry(
        client: MockDeviceClient<Mqtt<SqliteStore>>,
    ) -> (Telemetry<MockDeviceClient<Mqtt<SqliteStore>>>, TempDir) {
        let (dir, path) = temp_dir();

        (
            Telemetry {
                client,
                configs: HashMap::new(),
                tasks: TelemetryTasks::new(),
                file_state: FileStateRepository::new(&path, TELEMETRY_PATH),
                #[cfg(feature = "containers")]
                containers: std::sync::Arc::default(),
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

        let client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        let tel = Telemetry::from_config(
            client,
            &configs,
            t_dir,
            #[cfg(feature = "containers")]
            std::sync::Arc::default(),
        )
        .await;

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

        let client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        let mut tel = Telemetry::from_config(
            client,
            &configs,
            t_dir.clone(),
            #[cfg(feature = "containers")]
            std::sync::Arc::default(),
        )
        .await;

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
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_clone()
            .times(2)
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::new);

        let mut tel = Telemetry::from_config(
            client,
            &configs,
            t_dir.clone(),
            #[cfg(feature = "containers")]
            std::sync::Arc::default(),
        )
        .await;

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
        let client = {
            let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
            let mut seq = Sequence::new();

            client
                .expect_set_property()
                .once()
                .in_sequence(&mut seq)
                .with(
                    predicate::eq("io.edgehog.devicemanager.OSInfo"),
                    predicate::eq("/osName"),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            client
                .expect_set_property()
                .once()
                .in_sequence(&mut seq)
                .with(
                    predicate::eq("io.edgehog.devicemanager.OSInfo"),
                    predicate::eq("/osVersion"),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            client
                .expect_set_property()
                .times(..)
                .with(
                    predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            mock_runtime_info_telemetry(&mut client, &mut seq);

            client
                .expect_send_object_with_timestamp()
                .times(..)
                .with(
                    predicate::eq("io.edgehog.devicemanager.StorageUsage"),
                    predicate::always(),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _, _| Ok(()));

            client
                .expect_set_property()
                .times(..)
                .with(
                    predicate::eq("io.edgehog.devicemanager.NetworkInterfaceProperties"),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            client
                .expect_set_property()
                .with(
                    predicate::eq("io.edgehog.devicemanager.SystemInfo"),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            client
                .expect_set_property()
                .with(
                    predicate::eq("io.edgehog.devicemanager.BaseImage"),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _| Ok(()));

            client
                .expect_send_object_with_timestamp()
                .with(
                    predicate::eq("io.edgehog.devicemanager.SystemStatus"),
                    predicate::eq("/systemStatus"),
                    predicate::always(),
                    predicate::always(),
                )
                .returning(|_, _, _, _| Ok(()));

            client
        };

        let (mut telemetry, _dir) = mock_telemetry(client);

        telemetry.initial_telemetry().await;
    }
}
