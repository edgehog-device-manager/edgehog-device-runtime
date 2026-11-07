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

use std::path::PathBuf;

use edgehog_config::ConfigFile;
use edgehog_config::ota::{RaucDbus as ConfigRaucDbus, Reboot as ConfigReboot};
use edgehog_config::sdk::AstarteLibrary;
use edgehog_proto::config::v1::config::Connection;
use edgehog_proto::config::v1::config_service_client::ConfigServiceClient;
use edgehog_proto::config::v1::ota_config::{RaucConfig, RaucDbus, Reboot};
use edgehog_proto::config::v1::{
    AstarteDeviceConfig, Config, ContainerConfig, FileTransferConfig, ForwarderConfig,
    MessageHubConfig, OtaConfig, PutRequest, TelemetryConfig,
};
use edgehog_proto::tonic::transport::{Channel, Endpoint};
use eyre::{Context, OptionExt};

#[derive(Debug, Clone, clap::Subcommand)]
pub(crate) enum Service {
    /// Send a new configuration file to the Edgehog service.
    #[clap(
        alias = "send-config",
        alias = "send_config",
        alias = "put",
        alias = "send"
    )]
    Set {
        /// Path to the new configuration file to apply.
        file: PathBuf,
    },
}

impl Service {
    pub(crate) async fn run(&self, config: &ConfigFile) -> eyre::Result<()> {
        match self {
            Service::Set { file } => {
                let new_config = ConfigFile::read(file)
                    .await?
                    .ok_or_eyre(format!("configuration file not found: {}", file.display()))?;

                let file_name = file
                    .file_name()
                    .ok_or_eyre("invalid file path")?
                    .to_str()
                    .ok_or_eyre("file name is not valid UTF-8")?
                    .to_string();

                let proto_config = convert_to_proto(new_config);

                let mut client = connect_client(config).await?;

                let request = PutRequest {
                    file_name,
                    persist: None,
                    config: Some(proto_config),
                };

                client
                    .put(request)
                    .await
                    .wrap_err("failed to send configuration to the Edgehog service")?;

                println!("Configuration successfully sent to the Edgehog service.");
                Ok(())
            }
        }
    }
}

async fn connect_client(config: &ConfigFile) -> eyre::Result<ConfigServiceClient<Channel>> {
    let listener = config
        .service
        .as_ref()
        .and_then(|s| s.listener.clone())
        .unwrap_or_default();

    let endpoint_str = match &listener {
        edgehog_config::service::Listener::Unix(path) => {
            format!("unix:{}", path.display())
        }
        edgehog_config::service::Listener::Socket(addr) => {
            format!("http://{addr}")
        }
    };

    let endpoint = Endpoint::from_shared(endpoint_str.clone())
        .wrap_err_with(|| format!("invalid gRPC endpoint: {endpoint_str}"))?;

    let channel = endpoint
        .connect()
        .await
        .wrap_err_with(|| format!("failed to connect to Edgehog service at {endpoint_str}"))?;

    Ok(ConfigServiceClient::new(channel))
}

pub(crate) fn convert_to_proto(config: ConfigFile) -> Config {
    let connection = match config.astarte_library {
        None => Connection::Unspecified,
        Some(AstarteLibrary::AstarteDeviceSdk) => Connection::LibraryDirect,
        Some(AstarteLibrary::AstarteMessageHub) => Connection::LibraryMessageHub,
    } as i32;

    let astarte_device_sdk = config.astarte_device_sdk.map(|sdk| AstarteDeviceConfig {
        pairing_url: sdk.pairing_url.map(|u| u.to_string()).unwrap_or_default(),
        realm: sdk.realm.unwrap_or_default(),
        device_id: sdk.device_id,
        credentials_secret: sdk.credentials_secret,
        pairing_token: sdk.pairing_token,
        ignore_ssl: sdk.ignore_ssl.map(|b| b.to_string()),
    });

    let astarte_message_hub = config.astarte_message_hub.map(|hub| MessageHubConfig {
        endpoint: hub.endpoint.map(|u| u.to_string()).unwrap_or_default(),
    });

    let containers = config.containers.map(|c| ContainerConfig {
        enabled: c.enabled,
        required: c.required,
        max_retries: c.max_retries.and_then(|r| u32::try_from(r).ok()),
    });

    let ota = config.ota.map(|ota| {
        let reboot = match ota.reboot {
            None => Reboot::Unspecified,
            Some(ConfigReboot::Default) => Reboot::Default,
            Some(ConfigReboot::External) => Reboot::External,
        } as i32;

        let rauc = ota.rauc.map(|rauc| {
            let dbus_socket = match rauc.dbus_socket {
                None => RaucDbus::Unspecified,
                Some(ConfigRaucDbus::System) => RaucDbus::System,
                Some(ConfigRaucDbus::Session) => RaucDbus::Session,
            } as i32;

            RaucConfig { dbus_socket }
        });

        OtaConfig {
            enabled: ota.enabled,
            reboot,
            streaming: ota.streaming,
            rauc,
        }
    });

    let file_transfer = config.file_transfer.map(|ft| FileTransferConfig {
        enabled: ft.enabled,
        storage_reserved: ft.storage_reserved.map(|p| u32::from(*p)),
    });

    let forwarder = config
        .forwarder
        .map(|f| ForwarderConfig { enabled: f.enabled });

    let telemetry_config = config
        .telemetry_config
        .map(|telemetries| {
            telemetries
                .into_iter()
                .map(|t| {
                    (
                        t.interface_name,
                        TelemetryConfig {
                            enabled: t.enabled,
                            period_secs: t.period,
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    Config {
        connection,
        astarte_device_sdk,
        astarte_message_hub,
        containers,
        ota,
        file_transfer,
        forwarder,
        telemetry_config,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use edgehog_config::container::ContainersConfig;
    use edgehog_config::file_transfer::{FileTransferConfig as CfgFt, Percentage};
    use edgehog_config::forwarder::ForwarderConfig as CfgFwd;
    use edgehog_config::ota::{OtaConfig as CfgOta, RaucConfig as CfgRauc};
    use edgehog_config::sdk::{DeviceSdkConfig, MessageHubConfig as CfgMsgHub};
    use edgehog_config::service::{Listener, ServiceConfig};
    use edgehog_config::telemetry::TelemetryInterfaceConfig;
    use edgehog_proto::config::v1::PutResponse;
    use edgehog_proto::config::v1::config_service_server::{ConfigService, ConfigServiceServer};
    use edgehog_proto::tonic::transport::Server;
    use edgehog_proto::tonic::{Request, Response, Status};
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Mutex;
    use url::Url;

    #[test]
    fn test_convert_to_proto_empty() {
        let config = ConfigFile::default();
        let proto = convert_to_proto(config);

        assert_eq!(proto.connection, Connection::Unspecified as i32);
        assert!(proto.astarte_device_sdk.is_none());
        assert!(proto.astarte_message_hub.is_none());
        assert!(proto.containers.is_none());
        assert!(proto.ota.is_none());
        assert!(proto.file_transfer.is_none());
        assert!(proto.forwarder.is_none());
        assert!(proto.telemetry_config.is_empty());
    }

    #[test]
    fn test_convert_to_proto_full() {
        let config = ConfigFile {
            astarte_library: Some(AstarteLibrary::AstarteDeviceSdk),
            astarte_device_sdk: Some(DeviceSdkConfig {
                realm: Some("test-realm".to_string()),
                device_id: Some("device-123".to_string()),
                credentials_secret: Some("secret".to_string()),
                pairing_token: Some("token".to_string()),
                pairing_url: Some(Url::parse("https://pairing.astarte.com").unwrap()),
                ignore_ssl: Some(true),
            }),
            astarte_message_hub: Some(CfgMsgHub {
                endpoint: Some(Url::parse("http://localhost:8883").unwrap()),
            }),
            containers: Some(ContainersConfig {
                enabled: Some(true),
                required: Some(false),
                max_retries: Some(5),
            }),
            service: None,
            ota: Some(CfgOta {
                enabled: Some(true),
                reboot: Some(ConfigReboot::External),
                streaming: Some(true),
                rauc: Some(CfgRauc {
                    dbus_socket: Some(ConfigRaucDbus::System),
                }),
            }),
            file_transfer: Some(CfgFt {
                enabled: Some(true),
                storage_dir: None,
                storage_reserved: Percentage::new(25),
            }),
            forwarder: Some(CfgFwd {
                enabled: Some(true),
            }),
            store_directory: None,
            download_directory: None,
            telemetry_config: Some(vec![
                TelemetryInterfaceConfig {
                    interface_name: "org.example.Tele1".to_string(),
                    enabled: Some(true),
                    period: Some(10),
                },
                TelemetryInterfaceConfig {
                    interface_name: "org.example.Tele2".to_string(),
                    enabled: Some(false),
                    period: None,
                },
            ]),
            #[expect(deprecated)]
            interfaces_directory: None,
        };

        let proto = convert_to_proto(config);

        assert_eq!(proto.connection, Connection::LibraryDirect as i32);
        let sdk = proto.astarte_device_sdk.unwrap();
        assert_eq!(sdk.realm, "test-realm");
        assert_eq!(sdk.device_id.as_deref(), Some("device-123"));
        assert_eq!(sdk.credentials_secret.as_deref(), Some("secret"));
        assert_eq!(sdk.pairing_token.as_deref(), Some("token"));
        assert_eq!(sdk.pairing_url, "https://pairing.astarte.com/");
        assert_eq!(sdk.ignore_ssl.as_deref(), Some("true"));

        let hub = proto.astarte_message_hub.unwrap();
        assert_eq!(hub.endpoint, "http://localhost:8883/");

        let containers = proto.containers.unwrap();
        assert_eq!(containers.enabled, Some(true));
        assert_eq!(containers.required, Some(false));
        assert_eq!(containers.max_retries, Some(5));

        let ota = proto.ota.unwrap();
        assert_eq!(ota.enabled, Some(true));
        assert_eq!(ota.reboot, Reboot::External as i32);
        assert_eq!(ota.streaming, Some(true));
        assert_eq!(ota.rauc.unwrap().dbus_socket, RaucDbus::System as i32);

        let ft = proto.file_transfer.unwrap();
        assert_eq!(ft.enabled, Some(true));
        assert_eq!(ft.storage_reserved, Some(25));

        let fwd = proto.forwarder.unwrap();
        assert_eq!(fwd.enabled, Some(true));

        assert_eq!(proto.telemetry_config.len(), 2);
        let t1 = &proto.telemetry_config["org.example.Tele1"];
        assert_eq!(t1.enabled, Some(true));
        assert_eq!(t1.period_secs, Some(10));
        let t2 = &proto.telemetry_config["org.example.Tele2"];
        assert_eq!(t2.enabled, Some(false));
        assert_eq!(t2.period_secs, None);
    }

    struct MockConfigService {
        received: Arc<Mutex<Option<PutRequest>>>,
    }

    #[async_trait]
    impl ConfigService for MockConfigService {
        async fn put(&self, request: Request<PutRequest>) -> Result<Response<PutResponse>, Status> {
            let mut received = self.received.lock().await;
            *received = Some(request.into_inner());
            Ok(Response::new(PutResponse {}))
        }
    }

    #[tokio::test]
    async fn test_service_run_e2e() {
        let temp_dir = TempDir::new().unwrap();
        let socket_path = temp_dir.path().join("edgehog.sock");

        let received = Arc::new(Mutex::new(None));
        let mock_service = MockConfigService {
            received: Arc::clone(&received),
        };

        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        let incoming = tokio_stream::wrappers::UnixListenerStream::new(listener);

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            Server::builder()
                .add_service(ConfigServiceServer::new(mock_service))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
        });

        // Give server a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let base_config = ConfigFile {
            service: Some(ServiceConfig {
                enabled: Some(true),
                listener: Some(Listener::Unix(socket_path)),
            }),
            ..Default::default()
        };

        let new_config_path = temp_dir.path().join("50-config.toml");
        tokio::fs::write(
            &new_config_path,
            r#"
astarte_library = "astarte-device-sdk"

[astarte_device_sdk]
realm = "test-realm"
device_id = "device-uuid-1234"
"#,
        )
        .await
        .unwrap();

        let cmd = Service::Set {
            file: new_config_path.clone(),
        };

        cmd.run(&base_config).await.expect("cmd.run should succeed");

        let req = received
            .lock()
            .await
            .take()
            .expect("request must be received");
        assert_eq!(req.file_name, "50-config.toml");
        let cfg = req.config.expect("config must be set");
        assert_eq!(cfg.connection, Connection::LibraryDirect as i32);
        assert_eq!(
            cfg.astarte_device_sdk.unwrap().device_id.as_deref(),
            Some("device-uuid-1234")
        );

        let _ = shutdown_tx.send(());
    }
}
