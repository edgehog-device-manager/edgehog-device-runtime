// This file is part of Edgehog.
//
// Copyright 2022 - 2025 SECO Mind Srl
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

use std::io::IsTerminal;

use clap::Parser;
use eyre::{eyre, OptionExt, WrapErr};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use edgehog_device_runtime::data::connect_store;
use edgehog_device_runtime::AstarteLibrary;

use self::cli::Cli;
use self::config::read_options;

mod cli;
mod config;

//Error code state not recoverable
#[allow(unused)]
const ENOTRECOVERABLE: i32 = 131;

const DEFAULT_LOG_DIRECTIVE: &str = concat!(env!("CARGO_PKG_NAME"), "=info");

#[tokio::main]
async fn main() -> stable_eyre::Result<()> {
    stable_eyre::install()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_ansi(std::io::stdout().is_terminal()))
        .with(
            EnvFilter::builder()
                .with_default_directive(DEFAULT_LOG_DIRECTIVE.parse()?)
                .from_env_lossy(),
        )
        .try_init()?;

    // Use ring as default crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| eyre!("failed to install default crypto provider"))?;

    #[cfg(feature = "systemd")]
    {
        let default_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
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

    info!(
        "Using {} as store directory",
        options.store_directory.display()
    );

    let store = connect_store(&options.store_directory).await?;

    let mut tasks = JoinSet::new();

    let client = match &options.astarte_library {
        AstarteLibrary::AstarteDeviceSdk => {
            let astarte_sdk_options = options
                .astarte_device_sdk
                .as_ref()
                .ok_or_eyre("couldn't get astarte options")?;

            astarte_sdk_options
                .connect(
                    &mut tasks,
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
                .connect(&mut tasks, store, &options.interfaces_directory)
                .await?
        }
    };

    let mut dm = edgehog_device_runtime::Runtime::new(&mut tasks, options, client).await?;

    tasks.spawn(async move {
        dm.run()
            .await
            .wrap_err("the Device Runtime encountered an unrecoverable error")
    });

    while let Some(res) = tasks.join_next().await {
        match res {
            Ok(Ok(())) => {
                info!("task exited");
            }
            Ok(Err(err)) => {
                error!(error = format!("err:#"), "task exited");

                return Err(err);
            }
            Err(err) if err.is_cancelled() => {
                debug!(error = %err, "task exited");
            }
            Err(err) => {
                error!(error = %err, "task exited");

                return Err(err).wrap_err("task failed");
            }
        }
    }

    Ok(())
}

#[cfg(feature = "systemd")]
// clippy warns about the deprecated type, even if the alternative is not present in the MSRV
#[allow(deprecated)]
fn systemd_panic_hook(panic_info: &std::panic::PanicInfo) {
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
