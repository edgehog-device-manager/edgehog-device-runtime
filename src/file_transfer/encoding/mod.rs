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

//! Encoding Reader and Writer

use std::io;
use std::path::{Path, PathBuf};
use std::task::Poll;

use async_tar::Builder;
use eyre::Context;
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::io::simplex;
use tracing::{error, instrument};

use crate::file_transfer::file_system::walk::Walk;

pub(crate) mod tar_gz;

/// Maximum buffer size for the stream
pub const MAX_BUF_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub(crate) enum Paths {
    File { base: PathBuf, file: PathBuf },
    Dir { base: PathBuf, dir: PathBuf },
}

impl Paths {
    #[instrument]
    pub(crate) async fn read(path: PathBuf) -> io::Result<Self> {
        let meta = tokio::fs::metadata(&path).await?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();

        if meta.is_dir() {
            Ok(Paths::Dir { base, dir: path })
        } else {
            Ok(Paths::File { base, file: path })
        }
    }
}

pub(crate) trait EncoderBuilder {
    type Encoder: AsyncWrite + Unpin + Send + Sync + 'static;

    fn build(self, writer: simplex::Sender) -> TarEncoder<Self::Encoder>;
}

#[derive(Debug)]
#[pin_project]
pub(crate) struct EncodedReader {
    #[pin]
    rx: simplex::Receiver,
    #[pin]
    handle: JoinHandle<eyre::Result<()>>,
    joined: bool,
}

impl EncodedReader {
    pub fn spawn<E>(paths: Paths, builder: E) -> Self
    where
        E: EncoderBuilder,
    {
        let (tx, rx) = simplex::new(MAX_BUF_SIZE);
        let mut encoder = builder.build(tx);

        let handle = tokio::task::spawn(async move {
            let res_encode = encoder
                .encode(paths)
                .await
                .inspect_err(|error| error!(%error, "couldn't create archive"));

            let res_finalize = encoder
                .finalize()
                .await
                .inspect_err(|error| error!(%error, "couldn't finalize archive"));

            res_encode.and(res_finalize)
        });

        Self {
            rx,
            handle,
            joined: false,
        }
    }
}

impl AsyncRead for EncodedReader {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.project();

        if !*this.joined {
            match this.handle.poll(cx) {
                Poll::Ready(Ok(Err(_)) | Err(_)) => {
                    *this.joined = true;

                    return Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
                }
                Poll::Ready(Ok(Ok(()))) => {
                    *this.joined = true;
                }
                Poll::Pending => {}
            }
        }

        this.rx.poll_read(cx, buf)
    }
}

pub(crate) struct TarEncoder<W>
where
    W: AsyncWrite + Unpin + Send + Sync,
{
    archive: Builder<W>,
}

impl<W> TarEncoder<W>
where
    W: AsyncWrite + Unpin + Send + Sync,
{
    pub(crate) fn new(writer: W) -> Self {
        let mut builder = Builder::new(writer);
        builder.follow_symlinks(false);
        builder.mode(async_tar::HeaderMode::Deterministic);

        Self { archive: builder }
    }

    #[instrument(skip(self))]
    pub(crate) async fn encode(&mut self, input: Paths) -> eyre::Result<()>
    where
        Self: Send + Sync,
    {
        match input {
            Paths::File { base, file } => {
                self.append(&base, &file).await?;
            }
            Paths::Dir { base, dir } => {
                let mut dir = Walk::new(dir);

                while let Some(item) = dir.next().await {
                    let item = item?;

                    self.append(&base, item.path()).await?;
                }
            }
        }

        Ok(())
    }

    #[instrument(skip(self, base_path))]
    pub(crate) async fn append(&mut self, base_path: &Path, path: &Path) -> eyre::Result<()> {
        let name = path.strip_prefix(base_path)?;

        self.archive
            .append_path_with_name(path, name)
            .await
            .wrap_err("couldn't add path to TAR")?;

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) async fn finalize(self) -> eyre::Result<()> {
        self.archive.into_inner().await?.shutdown().await?;

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TarEncoding;

impl EncoderBuilder for TarEncoding {
    type Encoder = simplex::Sender;

    fn build(self, writer: simplex::Sender) -> TarEncoder<Self::Encoder> {
        TarEncoder::new(writer)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use async_tar::Archive;
    use futures::StreamExt;
    use rstest::rstest;
    use tempdir::TempDir;
    use tokio::{
        fs::File,
        io::{AsyncWriteExt, BufReader},
    };

    use crate::file_transfer::{
        encoding::TarEncoder,
        file_system::walk::{Walk, tests::mk_dir_structure},
    };

    #[rstest]
    #[tokio::test]
    async fn tar_roundtrip(#[future] mk_dir_structure: TempDir) {
        let dir = mk_dir_structure.await;

        let mut walk = Walk::new(dir.path().join("foo"));
        let path = dir.path().join("foo.tar");

        let file = File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
            .unwrap();

        let mut writer = TarEncoder::new(file);

        let mut exp = Vec::new();

        while let Some(item) = walk.next().await {
            let item = item.unwrap();

            writer.append(dir.path(), item.path()).await.unwrap();

            exp.push(item.path().strip_prefix(dir.path()).unwrap().to_path_buf());
        }

        writer
            .archive
            .into_inner()
            .await
            .unwrap()
            .flush()
            .await
            .unwrap();

        let out = TempDir::new("out").unwrap();

        let file = File::open(&path).await.unwrap();

        let mut reader = Archive::new(BufReader::new(file)).entries().unwrap();

        while let Some(item) = reader.next().await {
            let mut item = item.unwrap();

            item.unpack_in(out.path()).await.unwrap();
        }

        let walk = Walk::new(out.path().join("foo"));
        let mut res: Vec<PathBuf> = walk
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .strip_prefix(out.path())
                    .unwrap()
                    .to_path_buf()
            })
            .collect()
            .await;

        res.sort();
        exp.sort();

        assert_eq!(res, exp);
    }
}
