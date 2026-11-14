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

use std::path::Path;

use async_trait::async_trait;
use edgehog_config::ConfigFile;
use edgehog_config::container::ContainersConfig;
use edgehog_config::file_transfer::{FileTransferConfig, Percentage};
use edgehog_config::forwarder::ForwarderConfig;
use edgehog_config::ota::{OtaConfig, RaucConfig, RaucDbus, Reboot};
use edgehog_config::sdk::{AstarteLibrary, DeviceSdkConfig, MessageHubConfig};
use edgehog_config::telemetry::TelemetryInterfaceConfig;
use edgehog_proto::config::v1::config::Connection;
use edgehog_proto::config::v1::config_service_server::ConfigService;
use edgehog_proto::config::v1::{
    self as proto, AstarteDeviceConfig, Config, PutRequest, PutResponse,
};
use eyre::{Context, OptionExt};
use tonic::{Request, Response, Status};
use tracing::error;
use url::Url;

use crate::service::EdgehogService;

#[async_trait]
impl ConfigService for EdgehogService {
    async fn put(&self, request: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
        let put = request.into_inner();

        let Some(config) = put.config else {
            error!("missing configuration");

            return Err(Status::invalid_argument("missing configuration"));
        };

        if put.file_name.is_empty() {
            error!("empty file name");

            return Err(Status::invalid_argument("file name must not be empty"));
        }

        let file_name = Path::new(&put.file_name);

        if let Some(parent) = file_name.parent()
            && !parent.as_os_str().is_empty()
        {
            error!("file name provided with parent");

            return Err(Status::invalid_argument("file name has a parent path"));
        }

        let Some(file_name) = file_name.file_stem() else {
            return Err(Status::invalid_argument("missing file name"));
        };

        let mut path = self.config.dir.join(file_name);
        path.as_mut_os_string().push(".toml");

        // This endpoint will only validate the shape of the data, but you can update the same
        // config multiple times.
        let config = convert(config).map_err(|error| {
            error!(%error, "invalid configuration file");

            Status::invalid_argument("invalid configuration file")
        })?;

        let content = toml::to_string(&config).map_err(|error| {
            error!(%error, "couldn't serialize configuration file");

            Status::internal("couln't serialize config file")
        })?;

        tokio::fs::create_dir_all(&self.config.dir)
            .await
            .map_err(|error| {
                error!(%error, "couldn't create configuration directory");

                Status::internal("couldn't create configuration directory")
            })?;

        tokio::fs::write(&path, &content).await.map_err(|error| {
            error!(%error,"couldn't persist file");

            Status::internal("couldn't persist configuration")
        })?;

        self.config.tx.send((path, config)).await.map_err(|error| {
            error!(%error, "couldn't send new configuration");

            Status::unavailable("couldn't send new configuraiton")
        })?;

        Ok(Response::new(PutResponse {}))
    }
}

fn convert(value: Config) -> eyre::Result<ConfigFile> {
    let connection = value.connection();

    let Config {
        connection: _,
        astarte_device_sdk,
        astarte_message_hub,
        containers,
        ota,
        file_transfer,
        forwarder,
        telemetry_config,
    } = value;

    let astarte_library = match connection {
        Connection::Unspecified => None,
        Connection::LibraryDirect => Some(AstarteLibrary::AstarteDeviceSdk),
        Connection::LibraryMessageHub => Some(AstarteLibrary::AstarteMessageHub),
    };

    let astarte_device_sdk = astarte_device_sdk
        .map(convert_astarte_device_sdk)
        .transpose()
        .wrap_err("failed to convert astarte_device_sdk")?;

    let astarte_message_hub = astarte_message_hub
        .map(convert_astarte_message_hub)
        .transpose()
        .wrap_err("failed to convert astarte_message_hub")?;

    let containers = containers.map(convert_containers);
    let ota = ota.map(convert_ota);

    let file_transfer = file_transfer
        .map(convert_file_transfer)
        .transpose()
        .wrap_err("failed to convert file_transfer")?;

    let forwarder = forwarder.map(convert_forwarder);

    let telemetry_config = if telemetry_config.is_empty() {
        None
    } else {
        let mut list: Vec<TelemetryInterfaceConfig> = telemetry_config
            .into_iter()
            .map(|(interface_name, t)| TelemetryInterfaceConfig {
                interface_name,
                enabled: t.enabled,
                period: t.period_secs,
            })
            .collect();
        list.sort_by(|a, b| a.interface_name.cmp(&b.interface_name));
        Some(list)
    };

    Ok(ConfigFile {
        astarte_library,
        astarte_device_sdk,
        astarte_message_hub,
        containers,
        service: None,
        ota,
        file_transfer,
        forwarder,
        store_directory: None,
        download_directory: None,
        telemetry_config,
        #[expect(deprecated)]
        interfaces_directory: None,
    })
}

fn convert_astarte_device_sdk(value: proto::AstarteDeviceConfig) -> eyre::Result<DeviceSdkConfig> {
    let AstarteDeviceConfig {
        pairing_url,
        realm,
        device_id,
        credentials_secret,
        pairing_token,
        ignore_ssl,
    } = value;

    let pairing_url = (!pairing_url.is_empty())
        .then(|| Url::parse(&pairing_url).wrap_err("invalid pairing URL"))
        .transpose()?;

    let ignore_ssl = ignore_ssl
        .map(|s| s.parse::<bool>().wrap_err("invalid ignore_ssl boolean"))
        .transpose()?;

    let realm = (!realm.is_empty()).then_some(realm);

    Ok(DeviceSdkConfig {
        realm,
        device_id: device_id.filter(|s| !s.is_empty()),
        credentials_secret: credentials_secret.filter(|s| !s.is_empty()),
        pairing_token: pairing_token.filter(|s| !s.is_empty()),
        pairing_url,
        ignore_ssl,
    })
}

fn convert_astarte_message_hub(value: proto::MessageHubConfig) -> eyre::Result<MessageHubConfig> {
    let proto::MessageHubConfig { endpoint } = value;

    let endpoint = (!endpoint.is_empty())
        .then(|| Url::parse(&endpoint).wrap_err("invalid message hub endpoint URL"))
        .transpose()?;

    Ok(MessageHubConfig { endpoint })
}

fn convert_containers(value: proto::ContainerConfig) -> ContainersConfig {
    let proto::ContainerConfig {
        enabled,
        required,
        max_retries,
    } = value;

    ContainersConfig {
        enabled,
        required,
        max_retries: max_retries.map(|r| usize::try_from(r).unwrap_or(usize::MAX)),
    }
}

fn convert_ota(value: proto::OtaConfig) -> OtaConfig {
    let proto::OtaConfig {
        enabled,
        reboot: _,
        streaming,
        rauc,
    } = value;

    let reboot = match value.reboot() {
        proto::ota_config::Reboot::Unspecified => None,
        proto::ota_config::Reboot::Default => Some(Reboot::Default),
        proto::ota_config::Reboot::External => Some(Reboot::External),
    };

    let rauc = rauc.map(|rauc| {
        let proto::ota_config::RaucConfig { dbus_socket: _ } = rauc;

        let dbus_socket = match rauc.dbus_socket() {
            proto::ota_config::RaucDbus::Unspecified => None,
            proto::ota_config::RaucDbus::System => Some(RaucDbus::System),
            proto::ota_config::RaucDbus::Session => Some(RaucDbus::Session),
        };

        RaucConfig { dbus_socket }
    });

    OtaConfig {
        enabled,
        reboot,
        streaming,
        rauc,
    }
}

fn convert_file_transfer(value: proto::FileTransferConfig) -> eyre::Result<FileTransferConfig> {
    let proto::FileTransferConfig {
        enabled,
        storage_reserved,
    } = value;

    let storage_reserved = storage_reserved
        .map(|val| {
            let val_u8 = u8::try_from(val).wrap_err("storage_reserved out of range")?;
            Percentage::new(val_u8).ok_or_eyre("storage_reserved must be between 0 and 100")
        })
        .transpose()
        .wrap_err("invalid storage_reserved percentage")?;

    Ok(FileTransferConfig {
        enabled,
        storage_dir: None,
        storage_reserved,
    })
}

fn convert_forwarder(forwarder: proto::ForwarderConfig) -> ForwarderConfig {
    ForwarderConfig {
        enabled: forwarder.enabled,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::service::config::ConfigState;
    use crate::service::tests::mock_service_with_config;

    use super::*;

    fn mock_proto_config() -> Config {
        let mut telemetry_config = HashMap::new();
        telemetry_config.insert(
            "com.example.Telemetry2".to_string(),
            proto::TelemetryConfig {
                enabled: Some(false),
                period_secs: None,
            },
        );
        telemetry_config.insert(
            "com.example.Telemetry1".to_string(),
            proto::TelemetryConfig {
                enabled: Some(true),
                period_secs: Some(60),
            },
        );

        proto::Config {
            connection: proto::config::Connection::LibraryDirect as i32,
            astarte_device_sdk: Some(proto::AstarteDeviceConfig {
                pairing_url: "https://astarte.example.com/pairing".to_string(),
                realm: "test-realm".to_string(),
                device_id: Some("device-uuid-1234".to_string()),
                credentials_secret: Some("secret-token-xyz".to_string()),
                pairing_token: Some("pairing-token-abc".to_string()),
                ignore_ssl: Some("true".to_string()),
            }),
            astarte_message_hub: None,
            containers: Some(proto::ContainerConfig {
                enabled: Some(true),
                required: Some(true),
                max_retries: Some(3),
            }),
            ota: Some(proto::OtaConfig {
                enabled: Some(true),
                reboot: proto::ota_config::Reboot::External as i32,
                streaming: Some(true),
                rauc: Some(proto::ota_config::RaucConfig {
                    dbus_socket: proto::ota_config::RaucDbus::System as i32,
                }),
            }),
            file_transfer: Some(proto::FileTransferConfig {
                enabled: Some(true),
                storage_reserved: Some(20),
            }),
            forwarder: Some(proto::ForwarderConfig {
                enabled: Some(true),
            }),
            telemetry_config,
        }
    }

    #[tokio::test]
    async fn config_service_put() {
        let store = TempDir::new().unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        let dir = store.path().join("config");
        let state = ConfigState::new(dir.clone(), tx);

        let service = mock_service_with_config(state);

        let config = mock_proto_config();

        let file_name = "50-config.toml";
        service
            .put(Request::new(PutRequest {
                file_name: file_name.to_string(),
                persist: None,
                config: Some(config),
            }))
            .await
            .unwrap();

        let (path, res) = rx.recv().await.unwrap();

        let exp_path = dir.join(file_name);

        assert_eq!(path, exp_path);

        let stored = ConfigFile::read(path).await.unwrap().unwrap();

        assert_eq!(stored, res);

        let toml_str = toml::to_string_pretty(&res).expect("should serialize to toml");

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn test_convert_full_config() {
        let config = mock_proto_config();

        let converted = convert(config).expect("conversion should succeed");

        let toml_str = toml::to_string_pretty(&converted).expect("should serialize to toml");

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn test_convert_message_hub_config() {
        let config = proto::Config {
            connection: proto::config::Connection::LibraryMessageHub as i32,
            astarte_device_sdk: None,
            astarte_message_hub: Some(proto::MessageHubConfig {
                endpoint: "http://localhost:8883".to_string(),
            }),
            containers: None,
            ota: None,
            file_transfer: None,
            forwarder: None,
            telemetry_config: HashMap::new(),
        };

        let converted = convert(config).expect("conversion should succeed");

        let toml_str = toml::to_string_pretty(&converted).expect("should serialize to toml");

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn test_convert_empty_config() {
        let config = proto::Config::default();

        let converted = convert(config).expect("conversion should succeed");

        assert_eq!(converted, ConfigFile::default());

        let toml_str = toml::to_string_pretty(&converted).expect("should serialize to toml");

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn test_convert_invalid_pairing_url() {
        let config = proto::Config {
            astarte_device_sdk: Some(proto::AstarteDeviceConfig {
                pairing_url: "not a valid url".to_string(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let _ = convert(config).unwrap_err();
    }

    #[test]
    fn test_convert_invalid_ignore_ssl() {
        let config = proto::Config {
            astarte_device_sdk: Some(proto::AstarteDeviceConfig {
                ignore_ssl: Some("not_a_bool".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let _ = convert(config).unwrap_err();
    }

    #[test]
    fn test_convert_invalid_message_hub_endpoint() {
        let config = proto::Config {
            astarte_message_hub: Some(proto::MessageHubConfig {
                endpoint: "invalid url".to_string(),
            }),
            ..Default::default()
        };

        let _ = convert(config).unwrap_err();
    }

    #[test]
    fn test_convert_invalid_storage_reserved() {
        let config = proto::Config {
            file_transfer: Some(proto::FileTransferConfig {
                storage_reserved: Some(101),
                ..Default::default()
            }),
            ..Default::default()
        };

        let _ = convert(config).unwrap_err();
    }
}
