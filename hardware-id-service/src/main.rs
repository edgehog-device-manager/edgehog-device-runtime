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

use base64::prelude::*;
use clap::{Parser, Subcommand};
use stable_eyre::eyre::{Context, OptionExt};
use std::path::PathBuf;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;
use zbus::interface;

const SERVICE_NAME: &str = "io.edgehog.Device";
const DMI_SERIAL_FILE_PATH: &str = "/sys/class/dmi/id/board_serial";
const DEFAULT_NAMESPACE: &str = "f79ad91f-c638-4889-ae74-9d001a3b4cf8";

#[derive(Debug, Parser)]
struct Cli {
    /// Use the session for the HardwareId
    #[arg(default_value = "false", global = true, short, long)]
    session: bool,

    /// From which source to get the Hardware ID
    #[clap(subcommand)]
    command: HardwareId,
}

#[derive(Debug, Clone, Subcommand)]
enum HardwareId {
    /// Retrieve hardware id from file
    FilePath {
        /// File containing the hardware ID
        path: PathBuf,
    },
    /// Retrieve the hardware id from "/sys/class/dmi/id/board_serial"
    DmiSerial,
    /// Retrieve hardware id from Kernel parameters in the form key=value
    KernelCmdLine {
        /// The name of the cmdline
        key: String,
    },
}

struct Device {
    hw_id: String,
}

#[interface(name = "io.edgehog.Device1")]
impl Device {
    // Get hardware id using starting parameter.
    fn get_hardware_id(&self, namespace: &str) -> String {
        let ns = if namespace.is_empty() {
            DEFAULT_NAMESPACE
        } else {
            namespace
        };

        let namespace = Uuid::parse_str(ns).unwrap();
        let uuid = Uuid::new_v5(&namespace, self.hw_id.as_bytes());

        BASE64_URL_SAFE_NO_PAD.encode(uuid.as_bytes())
    }
}

// Simple DBUS service that retrieves a machine specific id and publish it on a system channel
#[tokio::main]
async fn main() -> stable_eyre::Result<()> {
    let cli = Cli::parse();

    stable_eyre::install()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init()?;

    let hw_id = match cli.command {
        HardwareId::FilePath { path } => tokio::fs::read_to_string(&path)
            .await
            .wrap_err_with(|| format!("couldn't read file {}", path.display()))?
            .trim()
            .to_string(),
        HardwareId::DmiSerial => tokio::fs::read_to_string(&DMI_SERIAL_FILE_PATH)
            .await
            .wrap_err("couldn't read DMI file")?
            .trim()
            .to_string(),
        HardwareId::KernelCmdLine { key } => {
            let cmdline = procfs::cmdline()?;

            cmdline
                .iter()
                .find_map(|s| {
                    s.split_once('=')
                        .and_then(|(name, value)| (name == key).then_some(value))
                })
                .ok_or_eyre("kernel cmdline value not found")?
                .trim()
                .to_string()
        }
    };

    let builder = if cli.session {
        zbus::connection::Builder::session()?
    } else {
        zbus::connection::Builder::system()?
    };

    let _conn = builder
        .name(SERVICE_NAME)?
        .serve_at("/io/edgehog/Device", Device { hw_id })?
        .build()
        .await?;

    info!("Service {SERVICE_NAME} started");

    tokio::signal::ctrl_c().await?;

    Ok(())
}
