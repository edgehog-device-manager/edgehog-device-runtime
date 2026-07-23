// This file is part of Edgehog.
//
// Copyright 2023, 2026 SECO Mind Srl
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

use clap::Parser;
use color_eyre::eyre::Context;
use std::io::{IsTerminal, stdout};
use std::path::{Path, PathBuf};
use tracing::{debug, info, instrument, trace};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use walkdir::WalkDir;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Directory of the Protobuf definitions.
    #[arg(short, long)]
    protos: PathBuf,
    /// Output directory for the generated files.
    #[arg(short, long)]
    output: PathBuf,
}

fn main() -> eyre::Result<()> {
    let cli = Cli::parse();

    color_eyre::install()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_ansi(stdout().is_terminal()))
        .with(tracing_error::ErrorLayer::default())
        .with(
            EnvFilter::builder()
                .with_default_directive("proto_codegen=DEBUG".parse()?)
                .from_env()?,
        )
        .try_init()?;

    let protos_path = cli
        .protos
        .canonicalize()
        .wrap_err_with(|| format!("couldn't resolve path {}", cli.protos.display()))?;

    debug!(protos = %protos_path.display(), "using proto directory");

    let protos = find_protos(&protos_path)?;

    debug!(output = %cli.output.display(), "using output directory");

    if !cli.output.exists() {
        debug!(output = %cli.output.display(), "output dir doesn't exists, creating it");

        std::fs::create_dir_all(&cli.output).wrap_err_with(|| {
            format!("couldn't create output directory {}", cli.output.display())
        })?;
    }

    prost_build::Config::new()
        .out_dir(&cli.output)
        .compile_protos(&protos, &[protos_path])
        .wrap_err("couldn't compile proto definitions")?;

    info!("gRPC and Protobuf file compiled");

    Ok(())
}

#[instrument]
fn filter_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.file_type().is_file() {
        trace!("checking file extension");

        entry.path().extension().is_some_and(|ext| ext == "proto")
    } else {
        trace!("entry is not a file");

        true
    }
}

#[instrument]
fn find_protos(path: &Path) -> eyre::Result<Vec<PathBuf>> {
    WalkDir::new(path)
        .into_iter()
        .filter_entry(filter_entry)
        .filter(|res| {
            let Ok(entry) = res else { return true };

            entry.file_type().is_file()
        })
        .map(|res| {
            res.map(|entry| entry.into_path())
                .inspect(|path| trace!(proto = %path.display(), "found proto"))
                .wrap_err("coudln't find protos")
        })
        .collect()
}
