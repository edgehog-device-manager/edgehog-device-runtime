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

use astarte_sdk::types::AstarteType;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::panic;

use edgehog_device_runtime::e2e_test::{get_hardware_info, get_os_info, get_runtime_info};
use edgehog_device_runtime::{DeviceManager, DeviceManagerOptions};

#[derive(Serialize, Deserialize)]
struct AstartePayload<T> {
    data: T,
}

#[tokio::main]
async fn main() -> Result<(), edgehog_device_runtime::error::DeviceManagerError> {
    let orig_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        println!("Test failed");
        orig_hook(panic_info);
        std::process::exit(1);
    }));

    //Waiting for Astarte Cluster to be ready...
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let astarte_api_url = std::env::var("E2E_ASTARTE_API_URL").unwrap();
    let realm = std::env::var("E2E_REALM_NAME").unwrap();
    let device_id = std::env::var("E2E_DEVICE_ID").unwrap();
    let credentials_secret = std::env::var("E2E_CREDENTIALS_SECRET").unwrap();
    let pairing_url = astarte_api_url.to_owned() + "/pairing";
    let e2e_token: &str = &std::env::var("E2E_TOKEN").unwrap();

    let device_options = DeviceManagerOptions {
        realm: realm.to_owned(),
        device_id: Some(device_id.to_owned()),
        credentials_secret: Some(credentials_secret),
        pairing_url: pairing_url.to_string(),
        pairing_token: None,
        interfaces_directory: "./edgehog/astarte-interfaces".to_string(),
        store_directory: "".to_string(),
        download_directory: "".to_string(),
    };

    let mut dm = DeviceManager::new(device_options).await?;

    dm.init().await?;

    tokio::task::spawn(async move {
        dm.run().await;
    });

    //Waiting for Edgehog Device Runtime to be ready...
    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
    do_e2e_test(
        astarte_api_url.to_owned(),
        realm.to_owned(),
        device_id.to_owned(),
        e2e_token.to_owned(),
    )
    .await;

    println!("Tests completed successfully");

    Ok(())
}
pub struct Test {
    api_url: String,
    realm: String,
    device_id: String,
    e2e_token: String,
}

impl<'a> Test {
    pub async fn run<F, T>(&self, f: F, fn_name: &str)
    where
        F: Fn(String, String, String, String) -> T + 'static,
        T: Future<Output = ()> + 'static,
    {
        println!("Run {} ", fn_name);
        f(
            self.api_url.clone(),
            self.realm.clone(),
            self.device_id.clone(),
            self.e2e_token.clone(),
        )
        .await;
        println!("Test {} completed successfully", fn_name);
    }
}

async fn do_e2e_test(api_url: String, realm: String, device_id: String, e2e_token: String) {
    let test = Test {
        api_url,
        realm,
        device_id,
        e2e_token,
    };
    test.run(os_info_test, "os_info_test").await;
    test.run(hardware_info_test, "hardware_info_test").await;
    test.run(runtime_info_test, "runtime_info_test").await;
}

fn os_info_test(
    api_url: String,
    realm: String,
    device_id: String,
    e2e_token: String,
) -> impl Future<Output = ()> {
    async move {
        let os_info_from_lib = get_os_info().unwrap();
        let json_os_info = reqwest::Client::new()
            .get(format!(
                "{}/appengine/v1/{}/devices/{}/interfaces/io.edgehog.devicemanager.OSInfo",
                api_url, realm, device_id
            ))
            .header("Authorization", format!("Bearer {}", e2e_token))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        #[derive(Serialize, Deserialize)]
        struct OsInfo {
            osName: String,
            osVersion: String,
        }

        let os_info_from_astarte: AstartePayload<OsInfo> =
            serde_json::from_str(&json_os_info).unwrap();
        assert_eq!(
            AstarteType::String(os_info_from_astarte.data.osName),
            os_info_from_lib.get("/osName").unwrap().to_owned()
        );
        assert_eq!(
            AstarteType::String(os_info_from_astarte.data.osVersion),
            os_info_from_lib.get("/osVersion").unwrap().to_owned()
        );
    }
}

fn hardware_info_test(
    api_url: String,
    realm: String,
    device_id: String,
    e2e_token: String,
) -> impl Future<Output = ()> {
    async move {
        let hardware_info_from_lib = get_hardware_info().unwrap();
        let json_hardware_info = reqwest::Client::new()
            .get(format!(
                "{}/appengine/v1/{}/devices/{}/interfaces/io.edgehog.devicemanager.HardwareInfo",
                api_url, realm, device_id
            ))
            .header("Authorization", format!("Bearer {}", e2e_token))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        #[derive(Serialize, Deserialize)]
        struct HardwareInfo {
            cpu: Cpu,
            mem: Mem,
        }

        #[derive(Serialize, Deserialize)]
        struct Cpu {
            architecture: String,
            model: String,
            modelName: String,
            vendor: String,
        }

        #[derive(Serialize, Deserialize)]
        struct Mem {
            totalBytes: i64,
        }

        let hardware_info_from_astarte: AstartePayload<HardwareInfo> =
            serde_json::from_str(&json_hardware_info).unwrap();
        assert_eq!(
            AstarteType::String(hardware_info_from_astarte.data.cpu.architecture),
            hardware_info_from_lib
                .get("/cpu/architecture")
                .unwrap()
                .to_owned()
        );
        assert_eq!(
            AstarteType::String(hardware_info_from_astarte.data.cpu.model),
            hardware_info_from_lib.get("/cpu/model").unwrap().to_owned()
        );
        assert_eq!(
            AstarteType::String(hardware_info_from_astarte.data.cpu.modelName),
            hardware_info_from_lib
                .get("/cpu/modelName")
                .unwrap()
                .to_owned()
        );
        assert_eq!(
            AstarteType::String(hardware_info_from_astarte.data.cpu.vendor),
            hardware_info_from_lib
                .get("/cpu/vendor")
                .unwrap()
                .to_owned()
        );
        assert_eq!(
            AstarteType::LongInteger(hardware_info_from_astarte.data.mem.totalBytes),
            hardware_info_from_lib
                .get("/mem/totalBytes")
                .unwrap()
                .to_owned()
        );
    }
}

fn runtime_info_test(
    api_url: String,
    realm: String,
    device_id: String,
    e2e_token: String,
) -> impl Future<Output = ()> {
    async move {
        let runtime_info_from_lib = get_runtime_info().unwrap();
        let runtime_info_json = reqwest::Client::new()
            .get(format!(
                "{}/appengine/v1/{}/devices/{}/interfaces/io.edgehog.devicemanager.RuntimeInfo",
                api_url, realm, device_id
            ))
            .header("Authorization", format!("Bearer {}", e2e_token))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        #[derive(Serialize, Deserialize)]
        struct RuntimeInfo {
            environment: String,
            name: String,
            url: String,
            version: String,
        }

        let runtime_info_from_astarte: AstartePayload<RuntimeInfo> =
            serde_json::from_str(&runtime_info_json).unwrap();
        assert_eq!(
            AstarteType::String(runtime_info_from_astarte.data.environment),
            runtime_info_from_lib
                .get("/environment")
                .unwrap()
                .to_owned()
        );
        assert_eq!(
            AstarteType::String(runtime_info_from_astarte.data.name),
            runtime_info_from_lib.get("/name").unwrap().to_owned()
        );
        assert_eq!(
            AstarteType::String(runtime_info_from_astarte.data.url),
            runtime_info_from_lib.get("/url").unwrap().to_owned()
        );
        assert_eq!(
            AstarteType::String(runtime_info_from_astarte.data.version),
            runtime_info_from_lib.get("/version").unwrap().to_owned()
        );
    }
}
