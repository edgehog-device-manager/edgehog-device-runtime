// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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

use std::io::IsTerminal;

use clap::Parser;
use edgehog_device_runtime::tasks::Tasks;
use eyre::eyre;
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use edgehog_device_runtime::cli::Cli;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    init_tracing()?;

    // Set default crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| eyre!("failed to install default crypto provider"))?;

    #[cfg(all(feature = "systemd", target_os = "linux"))]
    {
        let default_panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            systemd_panic_hook(panic_info);

            default_panic_hook(panic_info);
        }));
    }

    let cli = Cli::parse();

    let mut tasks = Tasks::init(cli).await?;

    tasks.spawn().await?;

    tasks.join().await?;

    Ok(())
}

fn init_tracing() -> eyre::Result<()> {
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(!cfg!(windows) && std::io::stdout().is_terminal())
        .with_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        );

    let subscribers = tracing_subscriber::registry()
        .with(layer)
        .with(ErrorLayer::default());

    #[cfg(feature = "security-events")]
    let subscribers = subscribers.with(
        tracing_journald::layer()?
            .with_syslog_identifier("edgehog_runtime_security_events".to_string())
            .with_filter(
                tracing_subscriber::filter::Targets::new()
                    .with_target("edgehog-security-event", tracing::Level::TRACE)
                    .with_target("security-event", tracing::Level::TRACE),
            ),
    );

    subscribers.try_init()?;

    Ok(())
}

#[cfg(all(feature = "systemd", target_os = "linux"))]
fn systemd_panic_hook(panic_info: &std::panic::PanicHookInfo) {
    // Error code state not recoverable
    const ENOTRECOVERABLE: i32 = 131;

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

    let status = format!("{message} {location}");
    systemd_wrapper::systemd_notify_errno_status(ENOTRECOVERABLE, &status);
}
