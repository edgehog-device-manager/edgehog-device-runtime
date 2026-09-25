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

use std::env::VarError;

use astarte_device_sdk::rumqttc::tokio_rustls::rustls::crypto::aws_lc_rs;
use clap::Parser;
use eyre::eyre;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

use self::cli::Cli;
use self::send::ApiClient;

mod cli;
mod send;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let cli = Cli::parse();

    color_eyre::install()?;

    aws_lc_rs::default_provider()
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
            appengine_url,
            data,
            curl,
        } => {
            let client = ApiClient::new(&cli.astarte, token.clone(), appengine_url.clone())?;

            if *curl {
                client.print_curl(data).await?;
            } else {
                client.read(data).await?;
            }
        }
    }

    Ok(())
}
