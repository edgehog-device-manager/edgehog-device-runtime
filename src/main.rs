/*
 * This file is part of Edgehog.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use clap::Parser;
use std::fs;
#[cfg(feature = "systemd")]
use std::panic::{self, PanicInfo};
use std::path::Path;

use config::read_options;
use edgehog_device_runtime::error::DeviceManagerError;
use edgehog_device_runtime::AstarteLibrary;

mod config;

//Error code state not recoverable
#[allow(unused)]
const ENOTRECOVERABLE: i32 = 131;

#[derive(Debug, Parser)]
struct Cli {
    /// Override configuration file path
    #[clap(short, long)]
    configuration_file: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), DeviceManagerError> {
    env_logger::init();
    #[cfg(feature = "systemd")]
    {
        let default_panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            default_panic_hook(panic_info);
            systemd_panic_hook(panic_info);
        }));
    }
    let Cli {
        configuration_file: config_file_path,
    } = Parser::parse();

    let options = read_options(config_file_path)?;

    if !Path::new(&options.download_directory).exists() {
        fs::create_dir_all(&options.download_directory).map_err(|err| {
            DeviceManagerError::FatalError(
                "Unable to create OTA download directory. ".to_owned() + &err.to_string(),
            )
        })?;
    }

    if !Path::new(&options.store_directory).exists() {
        fs::create_dir_all(&options.store_directory).map_err(|err| {
            DeviceManagerError::FatalError(
                "Unable to create store directory. ".to_owned() + &err.to_string(),
            )
        })?;
    }

    match &options.astarte_library {
        AstarteLibrary::AstarteDeviceSDK => {
            use edgehog_device_runtime::data::astarte_device_sdk_lib::{
                astarte_map_options, AstarteDeviceSdkLib,
            };

            let astarte_options = astarte_map_options(&options).await?;
            let mut dm = edgehog_device_runtime::DeviceManager::new(
                options,
                AstarteDeviceSdkLib::new(astarte_options).await?,
            )
            .await?;

            dm.init().await?;

            dm.run().await;
        }
        AstarteLibrary::AstarteMessageHub => {
            use edgehog_device_runtime::data::astarte_message_hub_node::AstarteMessageHubNode;

            let interfaces_directory = options.interfaces_directory.clone();
            let astarte_message_hub_options = options
                .astarte_message_hub
                .clone()
                .expect("Unable to find MessageHub options");
            let mut dm = edgehog_device_runtime::DeviceManager::new(
                options,
                AstarteMessageHubNode::new(astarte_message_hub_options, interfaces_directory)
                    .await?,
            )
            .await?;

            dm.init().await?;

            dm.run().await;
        }
    };

    Ok(())
}

#[cfg(feature = "systemd")]
fn systemd_panic_hook(panic_info: &PanicInfo) {
    use edgehog_device_runtime::wrapper;

    let message = if let Some(panic_msg) = panic_info.payload().downcast_ref::<&str>() {
        panic_msg
    } else {
        "panic occurred"
    };

    let location = if let Some(location) = panic_info.location() {
        format!("in file '{}' at line {}", location.file(), location.line(),)
    } else {
        "".to_string()
    };

    let status = format!("{} {}", message, location);
    wrapper::systemd::systemd_notify_errno_status(ENOTRECOVERABLE, &status);
}
