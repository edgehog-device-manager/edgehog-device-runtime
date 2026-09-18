// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Loads and stores the configuration files.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use edgehog_config::{ConfigFile, Mergiable};
use eyre::{Context, OptionExt};
use futures::TryStreamExt;
use tokio_stream::wrappers::ReadDirStream;
use tracing::debug;

const CONFIG_NAME: &str = "edgehog";
const SYSTEM_CONFIG: &str = "/etc/edgehog/";
const CONFIG_FILE: &str = "config.toml";
const CONFIG_D: &str = "config.d";

/// Custom config argument
#[derive(Debug)]
pub enum CustomConfig {
    /// A path to ta TOML file
    File(PathBuf),
    /// A path to ta a directory
    Dir(PathBuf),
}

/// Merges a configuration with an override.
///
/// The value set in the override must take precedence.
pub trait Override {
    /// Merges the override into self.
    fn merge(&mut self, overrides: Self);
}

impl<T> Override for Option<T> {
    fn merge(&mut self, value: Self) {
        if value.is_some() {
            *self = value;
        }
    }
}

/// Cache for the configuration files after being loaded.
#[derive(Debug, Clone)]
pub struct ConfigRepository {
    /// Cli overrides
    overrides: ConfigFile,
    /// Config merging all the configuration files
    file_config: ConfigFile,
    /// Drop-in from the config.d directories
    config_d: BTreeMap<EntryPath, ConfigFile>,
    /// Dynamic configurations
    dynamic: BTreeMap<EntryPath, ConfigFile>,
}

impl ConfigRepository {
    pub(crate) fn with_overrides(overrides: ConfigFile) -> Self {
        Self {
            overrides,
            file_config: ConfigFile::default(),
            config_d: BTreeMap::new(),
            dynamic: BTreeMap::new(),
        }
    }

    /// Checks if there are any dynamic configs
    pub fn has_dynamic(&self) -> bool {
        !self.dynamic.is_empty()
    }

    /// Returns the store directory
    pub(crate) fn get_store_directory(&self) -> Option<&Path> {
        let mut store_dir = self.file_config.store_directory.as_deref();

        for entry in self.config_d.values() {
            store_dir.merge(entry.store_directory.as_deref())
        }

        store_dir.merge(self.overrides.store_directory.as_deref());

        store_dir
    }

    /// Reads the configurations files.
    ///
    /// The order of the configurations files is as follows:
    ///
    /// 1. System default configuration file (e.g. /etc/edgehog/config.toml)
    /// 2. User default configuration file (e.g. $HOME/.config/edgehog/config.toml)
    /// 3. All the drop-ins configuration from various locations sorted alphabetical with the
    ///    following stable order:
    ///    1. System directory: /etc/edgehog/config.d/*.toml
    ///    1. User directory:  $HOME/.config/edgehog/config.d/*.toml
    pub async fn read_configs(&mut self, custom: &Option<CustomConfig>) -> eyre::Result<()> {
        match custom {
            Some(CustomConfig::File(config_file)) => {
                debug!(path = %config_file.display(), "using custom config file");

                let custom = ConfigFile::read(config_file)
                    .await?
                    .ok_or_eyre("couldn't read custom config file")?;

                self.file_config = custom;
            }
            Some(CustomConfig::Dir(config_dir)) => {
                debug!(dir = %config_dir.display(), "using custom config directory");

                self.read_config_dir(config_dir).await?;
            }
            None => {
                // On windows we don't support system configuration files
                if cfg!(unix) {
                    debug!("reading system config dir");

                    self.read_config_dir(Path::new(SYSTEM_CONFIG)).await?;
                }

                if let Some(user_config_dir) = dirs::config_dir() {
                    debug!("reading user config dir");

                    // $HOME/.config/edgehog
                    let user_config_dir = user_config_dir.join(CONFIG_NAME);

                    self.read_config_dir(&user_config_dir).await?;
                }
            }
        }

        Ok(())
    }

    /// Reads the configs in the dynamic configuration directory
    pub async fn read_dynamic(&mut self, dynamic_config_dir: &Path) -> eyre::Result<()> {
        Self::append_configs(&mut self.dynamic, dynamic_config_dir).await?;

        Ok(())
    }

    /// Appends the configuration files to the vector
    async fn append_configs(
        configs: &mut BTreeMap<EntryPath, ConfigFile>,
        path: &Path,
    ) -> eyre::Result<()> {
        let dir = match tokio::fs::read_dir(path).await {
            Ok(dir) => dir,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(eyre::Report::new(err).wrap_err("couldn't read config.d directory"));
            }
        };

        ReadDirStream::new(dir)
            .map_err(|error| {
                eyre::Report::new(error).wrap_err("couldn't read configuration directory")
            })
            .try_filter_map(|entry| async move {
                let path = entry.path();

                if path.extension().is_none_or(|ext| ext != "toml") {
                    return Ok(None);
                }

                let file_type = entry.file_type().await.wrap_err("couldn't get file type")?;

                if !file_type.is_file() {
                    return Ok(None);
                }

                let Some(config) = ConfigFile::read(&path).await? else {
                    return Ok(None);
                };

                Ok(Some((path, config)))
            })
            .try_for_each(|(path, config)| {
                configs.insert(EntryPath(path), config);

                futures::future::ok(())
            })
            .await?;

        Ok(())
    }

    /// Reads a config.toml and config.d files in the directory.
    async fn read_config_dir(&mut self, dir: &Path) -> eyre::Result<()> {
        self.file_config
            .read_and_merge(dir.join(CONFIG_FILE))
            .await?;

        Self::append_configs(&mut self.config_d, &dir.join(CONFIG_D)).await?;

        Ok(())
    }

    /// Merges all configuration entries into a single one.
    pub(crate) fn resolve_configs(&self) -> ConfigFile {
        let mut config = self
            .iter_configd_entries()
            .map(|(_path, config)| config)
            .cloned()
            .fold(self.file_config.clone(), |mut acc, config_d| {
                acc.merge(config_d);

                acc
            });

        config.merge(self.overrides.clone());

        config
    }

    fn iter_configd_entries(&self) -> impl Iterator<Item = (&EntryPath, &ConfigFile)> {
        let mut files = self.config_d.iter().peekable();
        let mut dynamic = self.dynamic.iter().peekable();

        std::iter::from_fn(move || {
            let f = files.peek();
            let d = dynamic.peek();

            match (f, d) {
                (None, None) => None,
                (None, Some(_d)) => dynamic.next(),
                (Some(_f), None) => files.next(),
                (Some((f, _)), Some((d, _))) => match f.cmp(d) {
                    Ordering::Less => files.next(),
                    Ordering::Equal => files.next(),
                    Ordering::Greater => dynamic.next(),
                },
            }
        })
    }

    /// Add a dynamic config
    pub fn add_dynamic(&mut self, path: PathBuf, config: ConfigFile) {
        self.dynamic.insert(EntryPath(path), config);
    }
}

/// Sort order for the configurations
// TODO: use lexicographical sorting for numbers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EntryPath(PathBuf);

impl PartialOrd for EntryPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EntryPath {
    fn cmp(&self, other: &Self) -> Ordering {
        self.file_name().cmp(&other.file_name())
    }
}

impl Deref for EntryPath {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use edgehog_config::sdk::AstarteLibrary;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn list_config_files() -> eyre::Result<()> {
        let dir = TempDir::with_prefix("list_config_files")?;

        tokio::fs::write(dir.path().join("config.toml"), "").await?;

        let config_d = dir.path().join("config.d");
        tokio::fs::create_dir(&config_d).await?;

        tokio::fs::write(dir.path().join("config.toml"), "").await?;
        tokio::fs::write(config_d.join("10-low.toml"), "").await?;
        tokio::fs::write(config_d.join("50-medium.toml"), "").await?;
        tokio::fs::write(config_d.join("99-high.toml"), "").await?;

        let store = dir.path().join("storage");
        let storage_config = store.join("config");

        tokio::fs::create_dir_all(&storage_config).await?;

        tokio::fs::write(storage_config.join("60-dynamic.toml"), "").await?;

        let mut loader = ConfigRepository::with_overrides(ConfigFile::default());

        let custom_dir = CustomConfig::Dir(dir.path().to_path_buf());
        loader.read_configs(&Some(custom_dir)).await.unwrap();
        loader.read_dynamic(&storage_config).await.unwrap();

        let exp = [
            "config.d/10-low.toml",
            "config.d/50-medium.toml",
            "storage/config/60-dynamic.toml",
            "config.d/99-high.toml",
        ]
        .map(|path| dir.path().join(path));

        let files: Vec<PathBuf> = loader
            .iter_configd_entries()
            .map(|(path, _config)| &path.0)
            .cloned()
            .collect();

        assert_eq!(files, exp);

        Ok(())
    }

    #[test]
    fn update_dynamic_config() {
        let mut config = ConfigRepository::with_overrides(ConfigFile::default());
        let path = PathBuf::from("/foo");

        config.add_dynamic(
            path.clone(),
            ConfigFile {
                astarte_library: Some(AstarteLibrary::AstarteDeviceSdk),
                ..Default::default()
            },
        );

        let exp = ConfigFile {
            astarte_library: Some(AstarteLibrary::AstarteMessageHub),
            ..Default::default()
        };

        config.add_dynamic(path.clone(), exp.clone());

        assert_eq!(config.dynamic.len(), 1);

        assert_eq!(*config.dynamic.values().next().unwrap(), exp);
    }
}
