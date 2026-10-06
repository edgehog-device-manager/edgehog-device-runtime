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

//! Configuration File for the Edgehog Device Runtime.

#![warn(
    missing_docs,
    rustdoc::missing_crate_level_docs,
    clippy::dbg_macro,
    clippy::todo
)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use self::container::ContainersConfig;
use self::file_transfer::FileTransferConfig;
use self::forwarder::ForwarderConfig;
use self::ota::OtaConfig;
use self::sdk::{AstarteLibrary, DeviceSdkConfig, MessageHubConfig};
use self::service::ServiceConfig;
use self::telemetry::TelemetryInterfaceConfig;

pub mod container;
pub mod file_transfer;
pub mod forwarder;
pub mod ota;
pub mod sdk;
pub mod service;
pub mod telemetry;

/// Configuration file
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    /// Connection type to use to connect with Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_library: Option<AstarteLibrary>,
    /// Astarte device sdk options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_device_sdk: Option<DeviceSdkConfig>,
    /// Astarte message hub options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub astarte_message_hub: Option<MessageHubConfig>,
    /// Container service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containers: Option<ContainersConfig>,
    /// Local Edgehog service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<ServiceConfig>,
    /// Ota configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ota: Option<OtaConfig>,
    /// File transfer service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_transfer: Option<FileTransferConfig>,
    /// Edgehog device forwarder service configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarder: Option<ForwarderConfig>,
    /// Edgehog interface directory
    #[deprecated(note = "the interfaces are included in the binary")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interfaces_directory: Option<PathBuf>,
    /// Persistent store directory writable by Edgehog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_directory: Option<PathBuf>,
    /// Download directory for the OTA updates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_directory: Option<PathBuf>,
    /// Configuration for the telemetries to send to Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig>>,
}

impl Mergiable for ConfigFile {
    fn merge(&mut self, value: Self) {
        let Self {
            astarte_library,
            astarte_device_sdk,
            astarte_message_hub,
            containers,
            service,
            ota,
            file_transfer,
            forwarder,
            #[expect(deprecated)]
            interfaces_directory,
            store_directory,
            download_directory,
            telemetry_config,
        } = value;

        self.astarte_library = astarte_library.or(self.astarte_library);
        self.astarte_device_sdk.merge(astarte_device_sdk);
        self.astarte_message_hub.merge(astarte_message_hub);
        self.containers.merge(containers);
        self.service.merge(service);
        self.ota.merge(ota);
        self.file_transfer.merge(file_transfer);
        self.forwarder.merge(forwarder);
        #[expect(deprecated)]
        {
            self.interfaces_directory = interfaces_directory.or(self.interfaces_directory.take());
        }
        self.store_directory = store_directory.or(self.store_directory.take());
        self.download_directory = download_directory.or(self.download_directory.take());
        self.telemetry_config.merge(telemetry_config);
    }
}

/// Merges two config structs
pub trait Mergiable {
    /// Merges the value into self.
    ///
    /// The fields set in value will take precedence.
    fn merge(&mut self, value: Self);
}

impl<T: Mergiable> Mergiable for Option<T> {
    fn merge(&mut self, value: Self) {
        match (self, value) {
            (Some(s), Some(v)) => {
                s.merge(v);
            }
            (this @ None, Some(value)) => {
                *this = Some(value);
            }
            (Some(_), None) | (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    #[expect(deprecated)]
    pub(crate) fn mock_config_file() -> ConfigFile {
        ConfigFile {
            astarte_library: Some(AstarteLibrary::AstarteDeviceSdk),
            astarte_device_sdk: sdk::tests::mock_device_sdk_config(),
            astarte_message_hub: None,
            containers: container::tests::mock_containers_config(),
            service: service::tests::mock_service_config(),
            ota: ota::tests::mock_ota_config(),
            file_transfer: file_transfer::tests::mock_file_transfer_config(),
            forwarder: forwarder::tests::mock_forwarder_config(),
            interfaces_directory: None,
            store_directory: Some(PathBuf::from("/var/lib/edgehog")),
            download_directory: Some(PathBuf::from("/var/lib/edgehog/download")),
            telemetry_config: telemetry::tests::mock_telemetry_interface_config().map(|t| vec![t]),
        }
    }

    #[test]
    fn config_file_roundtrip() {
        let exp = mock_config_file();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ConfigFile = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn config_file_empty_roundtrip() {
        let exp = ConfigFile::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ConfigFile = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn config_file_msg_hub_roundtrip() {
        let exp = ConfigFile {
            astarte_library: Some(AstarteLibrary::AstarteMessageHub),
            astarte_device_sdk: None,
            astarte_message_hub: sdk::tests::mock_msg_hub_config(),
            ..mock_config_file()
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ConfigFile = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn option_merge() {
        let mut opt: Option<ContainersConfig> = Some(ContainersConfig {
            enabled: Some(true),
            required: None,
            max_retries: Some(1),
        });
        opt.merge(None);
        let exp = Some(ContainersConfig {
            enabled: Some(true),
            required: None,
            max_retries: Some(1),
        });
        assert_eq!(opt, exp);

        opt.merge(Some(ContainersConfig {
            enabled: Some(false),
            required: Some(true),
            max_retries: None,
        }));
        let exp = Some(ContainersConfig {
            enabled: Some(false),
            required: Some(true),
            max_retries: Some(1),
        });
        assert_eq!(opt, exp);

        let mut none_opt: Option<ContainersConfig> = None;
        none_opt.merge(Some(ContainersConfig {
            enabled: Some(true),
            required: None,
            max_retries: None,
        }));
        let exp = Some(ContainersConfig {
            enabled: Some(true),
            required: None,
            max_retries: None,
        });
        assert_eq!(none_opt, exp);

        let mut none_opt2: Option<ContainersConfig> = None;
        none_opt2.merge(None);
        let exp = None;
        assert_eq!(none_opt2, exp);
    }

    #[test]
    fn config_file_merge() {
        let mut base = mock_config_file();

        let override_cfg = ConfigFile {
            containers: Some(ContainersConfig {
                enabled: Some(false),
                required: None,
                max_retries: Some(10),
            }),
            telemetry_config: Some(vec![TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
                enabled: Some(false),
                period: None,
            }]),
            store_directory: Some(PathBuf::from("/opt/edgehog/store")),
            ..Default::default()
        };

        let exp = ConfigFile {
            astarte_library: Some(AstarteLibrary::AstarteDeviceSdk),
            astarte_device_sdk: sdk::tests::mock_device_sdk_config(),
            astarte_message_hub: None,
            containers: Some(ContainersConfig {
                enabled: Some(false),
                required: Some(true),
                max_retries: Some(10),
            }),
            service: service::tests::mock_service_config(),
            ota: ota::tests::mock_ota_config(),
            file_transfer: file_transfer::tests::mock_file_transfer_config(),
            forwarder: forwarder::tests::mock_forwarder_config(),
            #[expect(deprecated)]
            interfaces_directory: None,
            store_directory: Some(PathBuf::from("/opt/edgehog/store")),
            download_directory: Some(PathBuf::from("/var/lib/edgehog/download")),
            telemetry_config: Some(vec![TelemetryInterfaceConfig {
                interface_name: "io.edgehog.devicemanager.SystemStatus".to_string(),
                enabled: Some(false),
                period: Some(60),
            }]),
        };

        base.merge(override_cfg);

        assert_eq!(base, exp);
    }
}
