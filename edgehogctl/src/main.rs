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

use std::io::IsTerminal;

use clap::Parser;
use eyre::eyre;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use self::file_transfer::FileTransfer;

mod client;
mod file_transfer;

#[derive(Debug, clap::Parser)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Command {
    #[clap(alias = "ft")]
    FileTransfer {
        /// Direction of the transfer
        #[clap(subcommand)]
        command: FileTransfer,
    },
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let fmt = tracing_subscriber::fmt::layer()
        .with_ansi(cfg!(not(windows)) && std::io::stdout().is_terminal());

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

    match cli.command {
        Command::FileTransfer { command } => {
            command.transfer().await?;
        }
    }

    Ok(())
}
