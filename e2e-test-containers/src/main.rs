// This file is part of Edgehog.
//
// Copyright 2023, 2024, 2026 SECO Mind Srl
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

use std::{env::VarError, path::PathBuf};

use astarte_device_sdk::rumqttc::tokio_rustls::rustls;
use clap::Parser;
use eyre::eyre;
use reqwest::Url;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use self::cli::Cli;
use self::send::ApiClient;

mod cli;
mod send;

/// Configuration file
// TODO: share with edgehog
#[derive(Debug, Deserialize, Clone, Default)]
pub struct Config {
    pub astarte_device_sdk: Option<DeviceSdkArgs>,
    pub store_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceSdkArgs {
    /// The Astarte realm the device belongs to.
    pub realm: Option<String>,
    /// A unique ID for the device.
    pub device_id: Option<String>,
    /// The credentials secret used to authenticate with Astarte.
    pub credentials_secret: Option<String>,
    /// Token used to register the device.
    pub pairing_token: Option<String>,
    /// Url to the Astarte pairing API
    pub pairing_url: Option<Url>,
    /// Ignores SSL error from the Astarte broker.
    pub ignore_ssl: Option<bool>,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let cli = Cli::parse();

    color_eyre::install()?;

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| eyre!("couldn't install default crypto provider"))?;

    let filter = if std::env::var("RUST_LOG").is_err_and(|err| err == VarError::NotPresent) {
        "warn,edgehog_device_runtime_containers=debug".parse()?
    } else {
        EnvFilter::builder()
            .with_default_directive("warn".parse()?)
            .from_env_lossy()
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .try_init()?;

    match &cli.command {
        cli::Command::Send {
            token,
            data,
            curl,
            config,
        } => {
            let config = tokio::fs::read_to_string(config).await?;
            let config: Config = toml::from_str(&config)?;

            let client = ApiClient::new(config, token.clone())?;

            if *curl {
                client.print_curl(data).await?;
            } else {
                client.read(data).await?;
            }
        }
    }

    Ok(())
}
