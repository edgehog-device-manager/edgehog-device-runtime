// This file is part of Edgehog.
//
// Copyright 2023-2024 SECO Mind Srl
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

use std::env::VarError;

use astarte_device_sdk::{
    builder::DeviceBuilder,
    rumqttc::tokio_rustls::rustls::crypto::aws_lc_rs,
    store::SqliteStore,
    transport::mqtt::{Credential, Mqtt, MqttConfig},
    DeviceClient, EventLoop,
};
use clap::Parser;
use cli::AstarteConfig;
use eyre::{eyre, WrapErr};
use receive::receive;
use tokio::task::JoinSet;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use self::cli::Cli;
use self::send::ApiClient;

mod cli;
mod receive;
mod send;

async fn connect(
    astarte: &AstarteConfig,
    tasks: &mut JoinSet<color_eyre::Result<()>>,
) -> color_eyre::Result<DeviceClient<Mqtt<SqliteStore>>> {
    let mut mqtt_config = MqttConfig::new(
        &astarte.realm,
        &astarte.device_id,
        Credential::secret(astarte.credentials_secret.clone()),
        &astarte.pairing_url,
    );
    mqtt_config.ignore_ssl_errors();

    let (client, connection) = DeviceBuilder::new()
        .interface_directory(&astarte.interfaces_dir)?
        .store_dir(&astarte.store_dir)
        .await?
        .connection(mqtt_config)
        .build()
        .await?;

    tasks.spawn(async move {
        connection.handle_events().await?;
        Ok(())
    });

    Ok(client)
}

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
        cli::Command::Receive => {
            let mut tasks = JoinSet::new();

            let client = connect(&cli.astarte, &mut tasks).await?;

            tasks.spawn(async move { receive(client, &cli.astarte.store_dir).await });

            tasks.spawn(async {
                tokio::signal::ctrl_c().await?;

                Ok(())
            });

            while let Some(res) = tasks.join_next().await {
                match res {
                    Err(err) if !err.is_cancelled() => {
                        return Err(err).wrap_err("tsak failed ");
                    }
                    Err(_cancel) => {}
                    Ok(res) => {
                        return res.wrap_err("task returned an error");
                    }
                }

                tasks.abort_all();
            }
        }
    }

    Ok(())
}
