// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::Parser;
use edgehog_config::ConfigFile;
use eyre::{OptionExt, eyre};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use self::file_transfer::FileTransfer;
use self::jobs::Jobs;
use self::service::Service;

mod client;
mod file_transfer;
mod jobs;
mod service;

#[derive(Debug, clap::Parser)]
struct Cli {
    /// Path to the device configuration file.
    #[clap(long, global = true)]
    config: Option<PathBuf>,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Command {
    #[clap(alias = "ft")]
    FileTransfer {
        /// Direction of the transfer
        #[clap(subcommand)]
        command: Box<FileTransfer>,
    },
    #[clap(alias = "j")]
    Jobs {
        /// Storage directory for the database
        #[clap(long)]
        storage_dir: PathBuf,

        /// Jobs commands
        #[clap(subcommand)]
        command: Jobs,
    },
    #[clap(alias = "svc")]
    Service {
        /// Service commands
        #[clap(subcommand)]
        command: Service,
    },
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let fmt = tracing_subscriber::fmt::layer()
        .with_ansi(cfg!(not(windows)) && std::io::stderr().is_terminal())
        .with_writer(io::stderr);

    tracing_subscriber::registry()
        .with(fmt)
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing::Level::DEBUG.into())
                .from_env_lossy(),
        )
        .with(tracing_error::ErrorLayer::default())
        .try_init()?;

    // Set default crypto provider
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| eyre!("failed to install default crypto provider"))?;

    let config = match cli.config {
        Some(path) => ConfigFile::read(&path)
            .await?
            .ok_or_eyre("missing configuration file")?,
        None => ConfigFile::default(),
    };

    match cli.command {
        Command::FileTransfer { command } => {
            command.transfer().await?;
        }
        Command::Jobs {
            storage_dir,
            command,
        } => command.run(&storage_dir).await?,
        Command::Service { command } => command.run(&config).await?,
    }

    Ok(())
}
