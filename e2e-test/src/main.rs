/*
 * This file is part of Astarte.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *    http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use astarte_device_sdk::types::AstarteType;
use clap::Parser;
use color_eyre::eyre::{bail, eyre, OptionExt, WrapErr};
use log::{error, info};
use reqwest::Response;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;
use tempdir::TempDir;
use url::Url;

use edgehog_device_runtime::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
use edgehog_device_runtime::data::connect_store;
use edgehog_device_runtime::telemetry::{
    hardware_info::get_hardware_info, os_info::get_os_info, runtime_info::get_runtime_info,
};
use edgehog_device_runtime::{AstarteLibrary, DeviceManager, DeviceManagerOptions};

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
async fn retry<'a, F, T, U>(times: usize, mut f: F) -> color_eyre::Result<U>
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

    env_logger::init();

    let cli = Cli::parse();

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
    };

    let store = connect_store(store_path.path())
        .await
        .wrap_err("couldn't connect to the store")?;

    let (pub_sub, handle) = astarte_options
        .connect(
            store,
            &device_options.store_directory,
            &device_options.interfaces_directory,
        )
        .await
        .wrap_err("couldn't connect to astarte")?;

    let dm = DeviceManager::new(device_options, pub_sub, handle).await?;

    dm.init().await?;

    tokio::task::spawn(async move { dm.run().await });

    //Waiting for Edgehog Device Runtime to be ready...
    tokio::time::sleep(tokio::time::Duration::from_millis(2000)).await;

    os_info_test(&cli).await?;
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
                .wrap_err("call failed")?;

            reqwest::get(&pairing)
                .await
                .and_then(Response::error_for_status)
                .wrap_err("call failed")
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
        reqwest::Client::new()
            .get(&url)
            .bearer_auth(&cli.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .map_err(Into::into)
    })
    .await
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OsInfo {
    os_name: String,
    os_version: String,
}

async fn os_info_test(cli: &Cli) -> color_eyre::Result<()> {
    let mut os_info_from_lib = get_os_info().await?;
    let os_info_from_astarte: AstartePayload<OsInfo> =
        get_interface_data(cli, "io.edgehog.devicemanager.OSInfo").await?;

    let name = os_info_from_lib
        .remove("/osName")
        .ok_or_eyre("missing osName")?;
    assert_eq!(AstarteType::String(os_info_from_astarte.data.os_name), name);

    let version = os_info_from_lib
        .remove("/osVersion")
        .ok_or_eyre("missing os version")?;
    assert_eq!(
        AstarteType::String(os_info_from_astarte.data.os_version),
        version
    );

    Ok(())
}

#[derive(Serialize, Deserialize)]
struct HardwareInfo {
    cpu: Cpu,
    mem: Mem,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cpu {
    architecture: String,
    model: String,
    model_name: String,
    vendor: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Mem {
    total_bytes: i64,
}

async fn hardware_info_test(cli: &Cli) -> color_eyre::Result<()> {
    let mut hardware_info_from_lib = get_hardware_info()?;

    let hardware_info_from_astarte: AstartePayload<HardwareInfo> =
        get_interface_data(cli, "io.edgehog.devicemanager.HardwareInfo").await?;

    let arch = hardware_info_from_lib
        .remove("/cpu/architecture")
        .ok_or_eyre("couldn't get cpu arch")?;
    assert_eq!(
        AstarteType::String(hardware_info_from_astarte.data.cpu.architecture),
        arch
    );

    let model = hardware_info_from_lib
        .remove("/cpu/model")
        .ok_or_eyre("couldn't get cpu model")?;
    assert_eq!(
        AstarteType::String(hardware_info_from_astarte.data.cpu.model),
        model
    );

    let name = hardware_info_from_lib
        .remove("/cpu/modelName")
        .ok_or_eyre("couldn't get cpu model name")?;
    assert_eq!(
        AstarteType::String(hardware_info_from_astarte.data.cpu.model_name),
        name
    );

    let vendor = hardware_info_from_lib
        .remove("/cpu/vendor")
        .ok_or_eyre("couldn't get cpu vendor")?;
    assert_eq!(
        AstarteType::String(hardware_info_from_astarte.data.cpu.vendor),
        vendor
    );

    let total = hardware_info_from_lib
        .remove("/mem/totalBytes")
        .ok_or_eyre("coudln't get total memory")?;
    assert_eq!(
        AstarteType::LongInteger(hardware_info_from_astarte.data.mem.total_bytes),
        total
    );

    Ok(())
}

#[derive(Serialize, Deserialize)]
struct RuntimeInfo {
    environment: String,
    name: String,
    url: String,
    version: String,
}

async fn runtime_info_test(cli: &Cli) -> color_eyre::Result<()> {
    let mut runtime_info_from_lib = get_runtime_info()?;

    let runtime_info_from_astarte: AstartePayload<RuntimeInfo> =
        get_interface_data(cli, "io.edgehog.devicemanager.RuntimeInfo").await?;

    let env = runtime_info_from_lib
        .remove("/environment")
        .ok_or_eyre("coudln't get environment")?;
    assert_eq!(
        AstarteType::String(runtime_info_from_astarte.data.environment),
        env
    );

    let name = runtime_info_from_lib
        .remove("/name")
        .ok_or_eyre("couldn't get the name")?;
    assert_eq!(
        AstarteType::String(runtime_info_from_astarte.data.name),
        name
    );
    let url = runtime_info_from_lib
        .remove("/url")
        .ok_or_eyre("coudln't get the url")?;
    assert_eq!(AstarteType::String(runtime_info_from_astarte.data.url), url);

    let version = runtime_info_from_lib
        .remove("/version")
        .ok_or_eyre("couldn't get the version")?;
    assert_eq!(
        AstarteType::String(runtime_info_from_astarte.data.version),
        version
    );

    Ok(())
}
