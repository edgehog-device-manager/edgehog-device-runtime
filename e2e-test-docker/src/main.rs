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

use astarte_device_sdk::{
    builder::{DeviceBuilder, DeviceSdkBuild},
    store::SqliteStore,
    transport::mqtt::{Credential, MqttConfig},
    DeviceClient, EventLoop,
};
use clap::Parser;
use cli::AstarteConfig;
use receive::receive;
use tokio::task::JoinHandle;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use self::cli::Cli;
use self::send::ApiClient;

mod cli;
mod receive;
mod send;
mod simulate;

async fn connect(
    astarte: &AstarteConfig,
) -> color_eyre::Result<(
    DeviceClient<SqliteStore>,
    JoinHandle<Result<(), astarte_device_sdk::Error>>,
)> {
    let mut mqtt_config = MqttConfig::new(
        &astarte.realm,
        &astarte.device_id,
        Credential::secret(astarte.credentials_secret.clone()),
        &astarte.pairing_url,
    );
    mqtt_config.ignore_ssl_errors();

    // 3. Create the device instance
    let (client, mut connection) = DeviceBuilder::new()
        .interface_directory(&astarte.interfaces_dir)?
        .store_dir(&astarte.store_dir)
        .await?
        .connect(mqtt_config)
        .await?
        .build();

    let handle = tokio::spawn(async move { connection.handle_events().await });

    Ok((client, handle))
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let cli = Cli::parse();

    color_eyre::install()?;

    let filter = EnvFilter::builder()
        .with_default_directive("edgehog_device_runtime_docker=DEBUG".parse()?)
        .from_env()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(filter)
        .try_init()?;

    match &cli.command {
        cli::Command::Send {
            token,
            appengine_url,
            data,
        } => {
            let client = ApiClient::new(&cli.astarte, token.clone(), appengine_url.clone())?;

            client.read(data).await?;
        }
        cli::Command::Receive => {
            let (client, connection_handle) = connect(&cli.astarte).await?;

            let recv_handle = tokio::spawn(async move { receive(client).await });

            tokio::signal::ctrl_c().await?;

            recv_handle.abort();
            recv_handle.await??;
            connection_handle.abort();
            connection_handle.await??;
        }
        cli::Command::Simulate => {
            let (client, connection_handle) = connect(&cli.astarte).await?;

            simulate::simulate(client.clone()).await?;

            connection_handle.abort();
            connection_handle.await??;
        }
    }

    Ok(())
}
