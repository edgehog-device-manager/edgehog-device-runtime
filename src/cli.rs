// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
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

//! Command Line options and configurations

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use edgehog_device_runtime::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
use eyre::OptionExt;
use serde::Deserialize;
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
        msghub: MsgHubArgs,
        #[command(flatten)]
        shared: SharedArgs,
    },
}

#[derive(Debug, Args)]
pub struct SharedArgs {
    /// Directory containing the Astarte interfaces.
    #[arg(short, long, env = "EDGEHOG_INTERFACES_DIR")]
    pub interfaces_dir: Option<PathBuf>,
    /// Directory used to retain configurations and other persistent data.
    #[arg(short, long, env = "EDGEHOG_STORE_DIR")]
    pub store_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Args, Deserialize)]
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

impl TryFrom<DeviceSdkArgs> for AstarteDeviceSdkConfigOptions {
    type Error = eyre::Error;

    fn try_from(value: DeviceSdkArgs) -> Result<Self, Self::Error> {
        let realm = value.realm.ok_or_eyre("config is missing the realm")?;
        let pairing_url = value
            .pairing_url
            .ok_or_eyre("config is missing the pairing_url")?;

        Ok(Self {
            realm,
            device_id: value.device_id,
            credentials_secret: value.credentials_secret,
            pairing_url,
            pairing_token: value.pairing_token,
            ignore_ssl: value.ignore_ssl.unwrap_or_default(),
        })
    }
}

impl DeviceSdkArgs {
    pub fn merge(&mut self, other: Self) {
        self.realm.merge(other.realm);
        self.device_id.merge(other.device_id);
        self.credentials_secret.merge(other.credentials_secret);
        self.pairing_token.merge(other.pairing_token);
        self.pairing_url.merge(other.pairing_url);
        self.ignore_ssl.merge(other.ignore_ssl);
    }
}

#[cfg(feature = "message-hub")]
#[derive(Debug, Clone, Args, Deserialize)]
#[command(next_help_heading = "Message Hub Options")]
pub struct MsgHubArgs {
    /// The Endpoint of the Astarte Message Hub to connect to
    #[arg(long, short, env = "EDGEHOG_MSGHUB_ENDPOINT")]
    endpoint: Option<Url>,
}

#[cfg(feature = "message-hub")]
impl MsgHubArgs {
    pub fn merge(&mut self, other: Self) {
        self.endpoint.merge(other.endpoint);
    }
}

#[cfg(feature = "message-hub")]
impl TryFrom<MsgHubArgs>
    for edgehog_device_runtime::data::astarte_message_hub_node::AstarteMessageHubOptions
{
    type Error = eyre::Error;

    fn try_from(value: MsgHubArgs) -> Result<Self, Self::Error> {
        let endpoint = value
            .endpoint
            .ok_or_eyre("config is missing the endpoint")?;

        Ok(Self { endpoint })
    }
}

pub trait OverrideOption {
    fn merge(&mut self, value: Self);
}

impl<T> OverrideOption for Option<T> {
    fn merge(&mut self, value: Self) {
        if value.is_some() {
            *self = value;
        }
    }
}
