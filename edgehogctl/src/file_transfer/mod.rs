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

use std::fmt::Display;
use std::io::{Write, stdout};
use std::path::PathBuf;

use bytes::BytesMut;
use color_eyre::Section;
use edgehog::file_transfer::request::FileDigest;
use edgehog::file_transfer::stream::PipeStreamWriter;
use edgehog::io::digest::Digest;
use eyre::{OptionExt, ensure};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, instrument};
use url::Url;
use uuid::Uuid;

use crate::client::ApiData;

#[derive(Debug, Clone, clap::ValueEnum)]
enum Encoding {
    TarGz,
    Gz,
}

impl Encoding {
    fn to_endpoint_value(&self) -> &'static str {
        match self {
            Encoding::TarGz => "tar.gz",
            Encoding::Gz => "gz",
        }
    }
}

#[derive(Debug, Default, Clone, clap::ValueEnum)]
enum Target {
    #[default]
    Storage,
    Stream,
    Filesystem,
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Storage => write!(f, "storage"),
            Target::Stream => write!(f, "streaming"),
            Target::Filesystem => write!(f, "filesystem"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerToDevice {
    id: Uuid,
    url: Url,
    http_header_keys: Vec<String>,
    http_header_values: Vec<String>,
    encoding: String,
    ttl_seconds: i64,
    file_mode: i64,
    user_id: i64,
    group_id: i64,
    progress: bool,
    digest: String,
    file_size_bytes: u64,
    destination_type: String,
    destination: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceToServer {
    id: Uuid,
    url: Url,
    http_header_keys: Vec<String>,
    http_header_values: Vec<String>,
    encoding: String,
    progress: bool,
    source_type: String,
    source: String,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub(crate) enum FileTransfer {
    Download(Download),
    Upload(Upload),
    Storage {
        /// Storage path
        storage: PathBuf,
    },
    /// Write to stdout the content of the passed file formatted as a valid
    /// stream. A valid file transfer stream
    /// contains a footer and a header as described in [`edgehog_device_runtime::file_transfer::stream`].
    FormatStream {
        /// Path of the file that will be converted to valid
        /// file transfer stream.
        file: PathBuf,
    },
}

impl FileTransfer {
    #[instrument(skip_all)]
    pub(crate) async fn transfer(self) -> eyre::Result<()> {
        match self {
            FileTransfer::Download(download) => download.transfer().await,
            FileTransfer::Upload(upload) => upload.transfer().await,
            FileTransfer::Storage { storage } => {
                let storage = storage.canonicalize()?;

                let start = storage.iter().count();

                let iter = walkdir::WalkDir::new(storage.join("file-store"));

                let mut stdout = stdout().lock();

                for entry in iter {
                    let entry = entry?;
                    let path = entry.path().canonicalize()?;

                    let dirs = path.iter().count().saturating_sub(start).saturating_sub(1);

                    let name = entry
                        .file_name()
                        .to_str()
                        .ok_or_eyre("couldn't get file name")?;

                    for _ in 0..dirs {
                        write!(stdout, "\t")?;
                    }

                    if entry.file_type().is_dir() {
                        writeln!(stdout, "{name}/")?;
                    } else {
                        writeln!(stdout, "{name}")?;
                    }
                }

                Ok(())
            }
            FileTransfer::FormatStream { file } => {
                let mut buf = BytesMut::with_capacity(8 * 1024);

                let file_digest = FileDigest::Sha256;
                // NOTE we keep an external digest to ensure that the it matches with the internal one
                let mut context = aws_lc_rs::digest::Context::from(file_digest);
                let mut reader = File::open(file).await?;
                let file_size = reader.metadata().await?.len();
                let writer = PipeStreamWriter::new(tokio::io::stdout(), file_size)
                    .write_header()
                    .await?;
                let mut writer = Digest::new(writer, file_digest);

                loop {
                    if reader.read_buf(&mut buf).await? == 0 {
                        break;
                    }

                    context.update(&buf);
                    writer.write_all_buf(&mut buf).await?;
                    buf.clear();
                }

                let (writer, status) = match writer.check_digest(context.finish().as_ref()) {
                    Ok(w) => (w, 0),
                    Err(w) => (w, 22),
                };

                writer.write_footer(status).await?;

                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Download {
    #[arg(long, default_value = "http://api.astarte.localhost")]
    astarte_url: Url,
    #[arg(long)]
    device_id: String,
    #[arg(long)]
    encoding: Option<Encoding>,
    #[arg(long)]
    progress: bool,
    #[arg(long)]
    file_size: Option<u64>,
    #[arg(long, default_value_t = Target::Storage)]
    destination_type: Target,
    #[arg(long)]
    destination: Option<String>,
    /// Url to download the file from
    url: Url,
    /// Path of the file on the file system to download
    path: PathBuf,
}

impl Download {
    #[instrument(skip_all)]
    pub(crate) async fn transfer(self) -> eyre::Result<()> {
        let content = tokio::fs::read(&self.path).await?;

        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &content);

        let encoding = self
            .encoding
            .map(|e| e.to_endpoint_value())
            .unwrap_or("")
            .to_string();

        let file_size_bytes = self.file_size.unwrap_or(u64::try_from(content.len())?);

        let data = ServerToDevice {
            id: Uuid::new_v4(),
            url: self.url,
            http_header_keys: Vec::new(),
            http_header_values: Vec::new(),
            encoding,
            ttl_seconds: 0,
            file_mode: 0,
            user_id: -1,
            group_id: -1,
            progress: self.progress,
            digest: format!("sha256:{}", hex::encode(digest.as_ref())),
            file_size_bytes,
            destination_type: self.destination_type.to_string(),
            destination: self.destination.unwrap_or_default(),
        };

        let client = reqwest::Client::builder()
            .use_preconfigured_tls(edgehog_tls::config()?)
            .build()?;

        let url = self
            .astarte_url
            .join(&format!("/appengine/v1/test/devices/{}/interfaces/io.edgehog.devicemanager.fileTransfer.ServerToDevice/request", self.device_id))?;

        let token = tokio::process::Command::new("astartectl")
            .args(["utils", "gen-jwt", "all-realm-apis"])
            .output()
            .await?;

        ensure!(token.status.success(), "error in astartectl command");

        let token = str::from_utf8(&token.stdout)?.trim();

        let resp = client
            .post(url)
            .bearer_auth(token)
            .json(&ApiData { data })
            .send()
            .await?;

        if let Err(err) = resp.error_for_status_ref() {
            let resp: serde_json::Value = resp.json().await?;

            let body = serde_json::to_string_pretty(&resp)?;

            return Err(err).with_note(|| body);
        }

        info!("request sent to device");

        Ok(())
    }
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct Upload {
    #[arg(long, default_value = "http://api.astarte.localhost")]
    astarte_url: Url,
    #[arg(long)]
    device_id: String,
    #[arg(long)]
    encoding: Option<Encoding>,
    #[arg(long)]
    progress: bool,
    #[arg(long, default_value_t)]
    source_type: Target,
    #[arg(long)]
    source: Option<String>,
    /// Url to download the file from
    url: Url,
}

impl Upload {
    #[instrument(skip_all)]
    pub(crate) async fn transfer(self) -> eyre::Result<()> {
        let encoding = self
            .encoding
            .map(|e| e.to_endpoint_value())
            .unwrap_or("")
            .to_string();

        let id = Uuid::new_v4();

        let data = DeviceToServer {
            id,
            url: self.url,
            http_header_keys: Vec::new(),
            http_header_values: Vec::new(),
            encoding,
            progress: self.progress,
            source_type: self.source_type.to_string(),
            source: self.source.unwrap_or_default(),
        };

        let client = reqwest::Client::builder()
            .use_preconfigured_tls(edgehog_tls::config()?)
            .build()?;

        let url = self
            .astarte_url
            .join(&format!("/appengine/v1/test/devices/{}/interfaces/io.edgehog.devicemanager.fileTransfer.DeviceToServer/request", self.device_id))?;

        let token = tokio::process::Command::new("astartectl")
            .args(["utils", "gen-jwt", "all-realm-apis"])
            .output()
            .await?;

        ensure!(token.status.success(), "error in astartectl command");

        let token = str::from_utf8(&token.stdout)?.trim();

        let resp = client
            .post(url)
            .bearer_auth(token)
            .json(&ApiData { data })
            .send()
            .await?;

        if let Err(err) = resp.error_for_status_ref() {
            let resp: serde_json::Value = resp.json().await?;

            let body = serde_json::to_string_pretty(&resp)?;

            return Err(err).with_note(|| body);
        }

        info!(%id, "request sent to device");

        Ok(())
    }
}
