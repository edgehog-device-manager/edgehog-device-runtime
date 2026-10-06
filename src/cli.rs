// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

//! Command Line options and configurations

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use edgehog_config::sdk::{AstarteLibrary, DeviceSdkConfig};
use edgehog_config::{ConfigFile, Mergiable};
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    display_name = "Edgehog Device Runtime",
    long_about = env!("CARGO_PKG_DESCRIPTION"),
    version,
    args_conflicts_with_subcommands = true,
)]
pub struct Cli {
    /// Override configuration file path
    #[arg(hide = true, long, global = true)]
    pub configuration_file: Option<PathBuf>,
    /// Override configuration file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub device: Option<DeviceSdkArgs>,

    #[command(flatten)]
    pub shared: Option<SharedArgs>,
}

impl From<Cli> for ConfigFile {
    fn from(value: Cli) -> Self {
        let Cli {
            configuration_file: _,
            config: _,
            command,
            device,
            shared,
        } = value;

        let mut config = ConfigFile::default();

        if let Some(shared) = shared {
            let SharedArgs {
                #[expect(deprecated)]
                interfaces_dir,
                store_dir,
            } = shared;

            config.store_directory = store_dir.or(config.store_directory.take());
            #[expect(deprecated)]
            {
                config.interfaces_directory = interfaces_dir.or(config.interfaces_directory.take());
            }
        }

        let shared = match command {
            Some(Command::DeviceSdk { device, shared }) => {
                config.astarte_library = Some(AstarteLibrary::AstarteDeviceSdk);
                config.astarte_device_sdk = Some(DeviceSdkConfig::from(device));

                Some(shared)
            }
            #[cfg(feature = "message-hub")]
            Some(Command::MsgHub { msghub, shared }) => {
                config.astarte_library = Some(AstarteLibrary::AstarteMessageHub);
                config.astarte_message_hub =
                    Some(edgehog_config::sdk::MessageHubConfig::from(msghub));

                Some(shared)
            }
            None => None,
        };

        if let Some(shared) = shared {
            let SharedArgs {
                #[expect(deprecated)]
                interfaces_dir,
                store_dir,
            } = shared;

            config.store_directory = store_dir.or(config.store_directory.take());
            #[expect(deprecated)]
            {
                config.interfaces_directory = interfaces_dir.or(config.interfaces_directory.take());
            }
        }

        if let Some(device) = device {
            config
                .astarte_device_sdk
                .merge(Some(DeviceSdkConfig::from(device)));
        }

        config
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Connects to Astarte directly using the Device SDK
    DeviceSdk {
        #[command(flatten)]
        device: DeviceSdkArgs,
        #[command(flatten)]
        shared: SharedArgs,
    },
    #[cfg(feature = "message-hub")]
    /// Connects to Astarte through the Message Hub
    MsgHub {
        #[command(flatten)]
        msghub: self::message_hub::MsgHubArgs,
        #[command(flatten)]
        shared: SharedArgs,
    },
}

#[derive(Debug, Args)]
pub struct SharedArgs {
    /// Directory containing the Astarte interfaces.
    #[arg(short, long, env = "EDGEHOG_INTERFACES_DIR")]
    #[deprecated(note = "the interfaces are included in the binary")]
    pub interfaces_dir: Option<PathBuf>,
    /// Directory used to retain configurations and other persistent data.
    #[arg(short, long, env = "EDGEHOG_STORE_DIR")]
    pub store_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Device SDK Options")]
pub struct DeviceSdkArgs {
    /// The Astarte realm the device belongs to.
    #[arg(short, long, env = "EDGEHOG_REALM")]
    pub realm: Option<String>,
    /// A unique ID for the device.
    #[arg(short, long, env = "EDGEHOG_DEVICE_ID")]
    pub device_id: Option<String>,
    /// The credentials secret used to authenticate with Astarte.
    #[arg(
        long,
        conflicts_with = "pairing_token",
        env = "EDGEHOG_CREDENTIALS_SECRET"
    )]
    pub credentials_secret: Option<String>,
    /// Token used to register the device.
    #[arg(
        long,
        conflicts_with = "credentials_secret",
        env = "EDGEHOG_PAIRING_TOKEN"
    )]
    pub pairing_token: Option<String>,
    /// Url to the Astarte pairing API
    #[arg(long, short = 'u', env = "EDGEHOG_PAIRING_URL")]
    pub pairing_url: Option<Url>,
    /// Ignores SSL error from the Astarte broker.
    #[arg(long, env = "EDGEHOG_IGNORE_SSL")]
    pub ignore_ssl: Option<bool>,
}

impl From<DeviceSdkArgs> for DeviceSdkConfig {
    fn from(value: DeviceSdkArgs) -> Self {
        let DeviceSdkArgs {
            realm,
            device_id,
            credentials_secret,
            pairing_token,
            pairing_url,
            ignore_ssl,
        } = value;

        DeviceSdkConfig {
            realm,
            device_id,
            credentials_secret,
            pairing_token,
            pairing_url,
            ignore_ssl,
        }
    }
}

#[cfg(feature = "message-hub")]
pub mod message_hub {
    use edgehog_config::sdk::MessageHubConfig;

    use super::*;

    #[derive(Debug, Clone, Args)]
    #[command(next_help_heading = "Message Hub Options")]
    pub struct MsgHubArgs {
        /// The Endpoint of the Astarte Message Hub to connect to
        #[arg(long, short, env = "EDGEHOG_MSGHUB_ENDPOINT")]
        endpoint: Option<Url>,
    }

    impl From<MsgHubArgs> for MessageHubConfig {
        fn from(value: MsgHubArgs) -> Self {
            let MsgHubArgs { endpoint } = value;

            MessageHubConfig { endpoint }
        }
    }
}
