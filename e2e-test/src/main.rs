// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use astarte_device_sdk::rumqttc::tokio_rustls::rustls;
use clap::Parser;
use color_eyre::eyre::{bail, eyre, OptionExt, WrapErr};
use edgehog_device_runtime::telemetry::hardware_info::HardwareInfo;
use edgehog_device_runtime::telemetry::os_release::{OsInfo, OsRelease};
use edgehog_device_runtime::telemetry::runtime_info::{RuntimeInfo, RUNTIME_INFO};
use reqwest::Response;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tempdir::TempDir;
use tokio::task::JoinSet;
use tracing::level_filters::LevelFilter;
use tracing::{debug, error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use url::Url;

use edgehog_device_runtime::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
use edgehog_device_runtime::data::connect_store;
use edgehog_device_runtime::{AstarteLibrary, DeviceManagerOptions, Runtime};

#[derive(Serialize, Deserialize)]
struct AstartePayload<T> {
    data: T,
}

#[derive(Debug, Parser)]
struct Cli {
    /// Astarte API base url
    #[arg(short = 'u', long, env = "E2E_ASTARTE_API_URL")]
    api_url: Url,
    /// Astarte realm to use
    #[arg(short, long, env = "E2E_REALM_NAME")]
    realm: String,
    /// Astarte device id to send data from.
    #[arg(short, long, env = "E2E_DEVICE_ID")]
    device_id: String,
    /// The test device credentials secret
    #[arg(short, long, env = "E2E_CREDENTIALS_SECRET")]
    secret: String,
    /// Token with access to the Astarte APIs
    #[arg(short, long, env = "E2E_TOKEN")]
    token: String,
    /// Ignore SSL errors when talking to MQTT Broker.
    #[arg(long, env = "E2E_IGNORE_SSL")]
    ignore_ssl: bool,
    /// Interface directory for the Device.
    #[arg(short, long, env = "E2E_INTERFACE_DIR")]
    interface_dir: PathBuf,
}

impl Cli {
    fn pairing_url(&self) -> color_eyre::Result<Url> {
        let mut url = self.api_url.clone();

        url.path_segments_mut()
            .map_err(|()| eyre!("couldn't get path for pairing url"))?
            .push("pairing");

        Ok(url)
    }
}

/// Retry the future multiple times
async fn retry<F, T, U>(times: usize, mut f: F) -> color_eyre::Result<U>
where
    F: FnMut() -> T,
    T: Future<Output = color_eyre::Result<U>>,
{
    let mut interval = tokio::time::interval(Duration::from_secs(1));

    for i in 1..=times {
        match (f)().await {
            Ok(o) => return Ok(o),
            Err(err) => {
                error!("failed retry {i} for: {err}");

                interval.tick().await;
            }
        }
    }

    bail!("to many attempts")
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::DEBUG.into())
                .from_env_lossy(),
        )
        .try_init()?;

    let cli = Cli::parse();

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| eyre!("couldn't install default crypto provider"))?;

    wait_for_cluster(&cli.api_url).await?;

    let pairing_url = cli.pairing_url()?;

    let store_path = TempDir::new("e2e-test").wrap_err("couldn't create temp directory")?;

    let astarte_options = AstarteDeviceSdkConfigOptions {
        realm: cli.realm.clone(),
        device_id: Some(cli.device_id.clone()),
        credentials_secret: Some(cli.secret.clone()),
        pairing_url,
        pairing_token: None,
        ignore_ssl: cli.ignore_ssl,
    };

    let device_options = DeviceManagerOptions {
        astarte_library: AstarteLibrary::AstarteDeviceSdk,
        astarte_device_sdk: Some(astarte_options.clone()),
        interfaces_directory: cli.interface_dir.clone(),
        store_directory: store_path.path().to_path_buf(),
        download_directory: store_path.path().join("downloads"),
        telemetry_config: Some(Vec::new()),
        #[cfg(feature = "message-hub")]
        astarte_message_hub: None,
        #[cfg(feature = "containers")]
        containers: edgehog_device_runtime::containers::ContainersConfig::default(),
    };

    let store = connect_store(store_path.path())
        .await
        .wrap_err("couldn't connect to the store")?;

    let mut tasks = JoinSet::new();

    let client = astarte_options
        .connect(
            &mut tasks,
            store,
            &device_options.store_directory,
            &device_options.interfaces_directory,
        )
        .await
        .wrap_err("couldn't connect to astarte")?;

    let mut dm = Runtime::new(&mut tasks, device_options, client).await?;

    tasks.spawn(async move {
        dm.run()
            .await
            .wrap_err("the Device Runtime encontered an unrecoverable error")
    });

    //Waiting for Edgehog Device Runtime to be ready...
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    let info = OsRelease::read()
        .await
        .ok_or_eyre("couldn't read os release")?;

    os_info_test(&cli, info.os_info).await?;
    hardware_info_test(&cli).await?;
    runtime_info_test(&cli).await?;

    info!("Tests completed successfully");

    Ok(())
}

async fn wait_for_cluster(api_url: &Url) -> color_eyre::Result<()> {
    let appengine = api_url.join("/appengine/health")?.to_string();
    let pairing = api_url.join("/pairing/health")?.to_string();

    retry(20, move || {
        let appengine = appengine.clone();
        let pairing = pairing.clone();

        async move {
            reqwest::get(&appengine)
                .await
                .and_then(Response::error_for_status)
                .wrap_err("appengine call failed")?;

            reqwest::get(&pairing)
                .await
                .and_then(Response::error_for_status)
                .wrap_err("pairing call failed")
        }
    })
    .await?;

    Ok(())
}

async fn get_interface_data<T>(cli: &Cli, interface: &str) -> color_eyre::Result<T>
where
    T: DeserializeOwned,
{
    let url = cli
        .api_url
        .join(&format!(
            "/appengine/v1/{}/devices/{}/interfaces/{interface}",
            cli.realm, cli.device_id
        ))?
        .to_string();

    retry(20, || async {
        let body = reqwest::Client::new()
            .get(&url)
            .bearer_auth(&cli.token)
            .send()
            .await
            .wrap_err_with(|| format!("get interface for {interface} failed"))?
            .error_for_status()?
            .text()
            .await
            .wrap_err("couldn't get body text")?;

        debug!("response: {body}");

        serde_json::from_str(&body)
            .wrap_err_with(|| format!("coudln't deserialize interface {interface}"))
    })
    .await
}

async fn os_info_test(cli: &Cli, info: OsInfo) -> color_eyre::Result<()> {
    let os_info_from_astarte: AstartePayload<OsInfo> =
        get_interface_data(cli, "io.edgehog.devicemanager.OSInfo").await?;

    let local_name = info.os_name.ok_or_eyre("missing osName")?;
    let astarte_name = os_info_from_astarte
        .data
        .os_name
        .ok_or_eyre("missing osName from Astarte")?;
    assert_eq!(local_name, astarte_name);

    let local_version = info.os_version.ok_or_eyre("missing osVersion")?;
    let astarte_version = os_info_from_astarte
        .data
        .os_version
        .ok_or_eyre("missing osVersion from Astarte")?;
    assert_eq!(local_version, astarte_version);

    Ok(())
}

async fn hardware_info_test(cli: &Cli) -> color_eyre::Result<()> {
    let local_hw = HardwareInfo::read().await;

    let astarte_hw = get_interface_data::<AstartePayload<HardwareInfo>>(
        cli,
        "io.edgehog.devicemanager.HardwareInfo",
    )
    .await?
    .data;

    assert_eq!(local_hw.cpu.architecture, astarte_hw.cpu.architecture);

    assert_eq!(local_hw.cpu.model, astarte_hw.cpu.model);

    assert_eq!(local_hw.cpu.model_name, astarte_hw.cpu.model_name);

    assert_eq!(local_hw.cpu.vendor, astarte_hw.cpu.vendor);

    assert_eq!(local_hw.mem.total_bytes, astarte_hw.mem.total_bytes);

    Ok(())
}

async fn runtime_info_test(cli: &Cli) -> color_eyre::Result<()> {
    let local_rt = RUNTIME_INFO;

    let astarte_rt = get_interface_data::<AstartePayload<RuntimeInfo>>(
        cli,
        "io.edgehog.devicemanager.RuntimeInfo",
    )
    .await?
    .data;

    assert_eq!(local_rt.environment, astarte_rt.environment);
    assert_eq!(local_rt.name, astarte_rt.name);
    assert_eq!(local_rt.url, astarte_rt.url);
    assert_eq!(local_rt.version, astarte_rt.version);

    Ok(())
}
