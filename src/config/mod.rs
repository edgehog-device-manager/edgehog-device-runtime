// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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

use std::io;
use std::path::Path;

use edgehog_config::sdk::AstarteLibrary;
use edgehog_config::{ConfigFile, Mergiable};
use eyre::{Context, ensure};
use tracing::{debug, info, trace};

use crate::DeviceManagerOptions;
use crate::cli::Cli;
use crate::repository::StateRepository;
use crate::repository::file_state_repository::FileStateRepository;

const PATHS: [&str; 2] = ["edgehog-config.toml", "/etc/edgehog/config.toml"];

/// Read configuration options
pub async fn read_options(cli: Cli) -> eyre::Result<DeviceManagerOptions> {
    if let Some(path) = &cli.config {
        ensure!(
            path.exists(),
            "configuration file {} doesn't exists",
            path.display()
        );
    }

    let override_config_file_path = cli.config.as_deref().or(cli.configuration_file.as_deref());

    let mut config = read_config_file(override_config_file_path).await?;

    config.merge(ConfigFile::from(cli));

    match config.astarte_library {
        Some(AstarteLibrary::AstarteDeviceSdk) | None => {
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            if config
                .astarte_device_sdk
                .as_ref()
                .is_none_or(|c| c.device_id.is_none())
            {
                let device_id = crate::data::device_sdk::hardware_id_from_dbus().await?;

                config.astarte_device_sdk.get_or_insert_default().device_id = device_id;
            }

            read_credential(&mut config).await?;
        }
        Some(AstarteLibrary::AstarteMessageHub) => {}
    }

    config.try_into()
}

async fn read_config_file(
    override_config_file_path: Option<&Path>,
) -> Result<ConfigFile, eyre::Error> {
    match override_config_file_path {
        Some(custom_config) => {
            debug!(config = %custom_config.display(), "reading custom configuration file");

            let custom_config = tokio::fs::read_to_string(custom_config)
                .await
                .wrap_err("coudln't read custom configuration file")?;
            let config: ConfigFile = toml::from_str(&custom_config)
                .wrap_err("couldn't parse custom configuration file")?;

            Ok(config)
        }
        None => {
            for path in PATHS {
                match tokio::fs::read_to_string(path).await {
                    Ok(content) => {
                        let config_file: ConfigFile = toml::from_str(&content)
                            .wrap_err_with(|| format!("couldn't parse config file {path}"))?;

                        info!(path, "configuration file");

                        return Ok(config_file);
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        trace!(path, "file not found");
                    }
                    Err(error) => {
                        return Err(error)
                            .wrap_err_with(|| format!("couldn't read config file {path}"));
                    }
                }
            }

            debug!("no configuration file found");

            Ok(ConfigFile::default())
        }
    }
}

/// Reads the credentials secret
pub async fn read_credential(config: &mut ConfigFile) -> eyre::Result<()> {
    let has_credential_secret = config
        .astarte_device_sdk
        .as_ref()
        .is_some_and(|s| s.credentials_secret.as_ref().is_some());

    if has_credential_secret {
        info!("has credentials secret");

        return Ok(());
    }

    let Some(device_id) = config
        .astarte_device_sdk
        .as_ref()
        .and_then(|sdk| sdk.device_id.as_deref())
    else {
        debug!("missing device_id");

        return Ok(());
    };

    let Some(store_directory) = config.store_directory.as_deref() else {
        debug!("missing store directory");

        return Ok(());
    };

    let registry =
        FileStateRepository::new(store_directory, format!("credentials_{device_id}.json"));

    if !StateRepository::<String>::exists(&registry).await {
        trace!("no legacy state repository");

        return Ok(());
    }

    let credential_secret = registry
        .read()
        .await
        .wrap_err("couldn't read credential secret")?;

    info!("reading credentials_secret from state repository");

    config
        .astarte_device_sdk
        .get_or_insert_default()
        .credentials_secret = Some(credential_secret);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use edgehog_config::sdk::DeviceSdkConfig;
    use tempfile::TempDir;

    #[tokio::test]
    async fn get_credentials_secret_persistence_success() {
        let dir = TempDir::with_prefix("sdk_cred_persist_ok").unwrap();
        let path = dir.path();

        let device_id = "device_id";

        let full_path = path.join(format!("credentials_{device_id}.json"));

        let exp = "credential_secret";

        tokio::fs::write(&full_path, format!("\"{exp}\""))
            .await
            .unwrap();

        let mut config = ConfigFile {
            astarte_device_sdk: Some(DeviceSdkConfig {
                device_id: Some(device_id.to_string()),
                ..Default::default()
            }),
            store_directory: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        read_credential(&mut config).await.unwrap();

        let res = config
            .astarte_device_sdk
            .unwrap()
            .credentials_secret
            .unwrap();

        assert_eq!(res, exp);
    }
}
