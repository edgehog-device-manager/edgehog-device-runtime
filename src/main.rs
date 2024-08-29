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

#[cfg(feature = "systemd")]
use std::panic::{self, PanicInfo};

use clap::Parser;
use cli::Cli;
use log::warn;
use stable_eyre::eyre::{OptionExt, WrapErr};

use config::read_options;
use edgehog_device_runtime::data::connect_store;
use edgehog_device_runtime::AstarteLibrary;

mod cli;
mod config;

//Error code state not recoverable
#[allow(unused)]
const ENOTRECOVERABLE: i32 = 131;

#[tokio::main]
async fn main() -> stable_eyre::Result<()> {
    stable_eyre::install()?;
    env_logger::init();

    #[cfg(feature = "systemd")]
    {
        let default_panic_hook = panic::take_hook();
        panic::set_hook(Box::new(move |panic_info| {
            systemd_panic_hook(panic_info);

            default_panic_hook(panic_info);
        }));
    }

    let cli = Cli::parse();

    if cli.configuration_file.is_some() {
        warn!("the option --configuration-file is deprecated, please use --config instead")
    }

    let options = read_options(cli).await?;

    if !options.download_directory.exists() {
        tokio::fs::create_dir_all(&options.download_directory)
            .await
            .wrap_err("Unable to create OTA download directory.")?;
    }

    if !options.store_directory.exists() {
        tokio::fs::create_dir_all(&options.store_directory)
            .await
            .wrap_err("Unable to create store directory")?;
    }

    let store = connect_store(&options.store_directory).await?;

    let (pub_sub, handle) = match &options.astarte_library {
        AstarteLibrary::AstarteDeviceSdk => {
            let astarte_sdk_options = options
                .astarte_device_sdk
                .as_ref()
                .ok_or_eyre("couldn't get astarte options")?;

            astarte_sdk_options
                .connect(
                    store,
                    &options.store_directory,
                    &options.interfaces_directory,
                )
                .await?
        }
        #[cfg(feature = "message-hub")]
        AstarteLibrary::AstarteMessageHub => {
            let astarte_message_hub_options = options
                .astarte_message_hub
                .as_ref()
                .ok_or_eyre("couldn't get MessageHub options")?;

            astarte_message_hub_options
                .connect(store, &options.interfaces_directory)
                .await?
        }
    };

    let dm = edgehog_device_runtime::DeviceManager::new(options, pub_sub, handle).await?;

    dm.init().await?;

    dm.run().await?;

    Ok(())
}

#[cfg(feature = "systemd")]
fn systemd_panic_hook(panic_info: &PanicInfo) {
    use edgehog_device_runtime::systemd_wrapper;

    let message = if let Some(panic_msg) = panic_info.payload().downcast_ref::<&str>() {
        panic_msg
    } else {
        "panic occurred"
    };

    let location = if let Some(location) = panic_info.location() {
        format!("in file '{}' at line {}", location.file(), location.line())
    } else {
        "".to_string()
    };

    let status = format!("{} {}", message, location);
    systemd_wrapper::systemd_notify_errno_status(ENOTRECOVERABLE, &status);
}
