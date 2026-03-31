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

//! Reader and Writer for a `tar.gz` file

use std::path::Path;
use std::task::{Poll, ready};

use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use async_tar::{Archive, Builder, Entries};
use eyre::Context;
use futures::StreamExt;
use pin_project::pin_project;
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tokio_stream::Stream;
use tracing::instrument;

pub(crate) struct TarGzWriter<W>
where
    W: AsyncWrite + Unpin + Send + Sync,
{
    archive: Builder<GzipEncoder<W>>,
}

impl<W> TarGzWriter<W>
where
    W: AsyncWrite + Unpin + Send + Sync,
{
    pub(crate) fn new(writer: W) -> Self {
        Self {
            archive: Builder::new(GzipEncoder::new(writer)),
        }
    }

    pub(crate) async fn append(&mut self, base_path: &Path, path: &Path) -> eyre::Result<()> {
        let name = path.strip_prefix(base_path)?;

        self.archive
            .append_path_with_name(path, name)
            .await
            .wrap_err("couldn't add path to TAR")?;

        Ok(())
    }

    pub(crate) async fn finalize(self) -> eyre::Result<()> {
        self.archive.into_inner().await?.flush().await?;

        Ok(())
    }
}

#[pin_project]
pub(crate) struct TarGzReader<R>
where
    R: AsyncBufRead + Unpin,
{
    archive: Entries<GzipDecoder<R>>,
}

impl<R> TarGzReader<R>
where
    R: AsyncBufRead + Unpin,
{
    #[instrument(skip_all)]
    pub(crate) fn create(reader: R) -> eyre::Result<Self> {
        Archive::new(GzipDecoder::new(reader))
            .entries()
            .map(|archive| Self { archive })
            .wrap_err("couldn't read open entries")
    }
}

impl<R> Stream for TarGzReader<R>
where
    R: AsyncBufRead + Unpin,
{
    type Item = eyre::Result<async_tar::Entry<Archive<GzipDecoder<R>>>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let res = ready!(this.archive.poll_next_unpin(cx))
            .map(|res| res.wrap_err("couldn't get next entry"));

        Poll::Ready(res)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;
    use tempdir::TempDir;
    use tokio::fs::File;
    use tokio::io::{AsyncWriteExt, BufReader};

    use crate::file_transfer::file_system::walk::Walk;
    use crate::file_transfer::file_system::walk::tests::mk_dir_structure;

    use super::*;

    #[rstest]
    #[tokio::test]
    async fn tar_gz_roundtrip(#[future] mk_dir_structure: TempDir) {
        let dir = mk_dir_structure.await;

        let mut walk = Walk::new(dir.path().join("foo"));
        let path = dir.path().join("foo.tar.gz");

        let file = File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
            .unwrap();

        let mut writer = TarGzWriter::new(file);

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

        let mut reader = TarGzReader::create(BufReader::new(file)).unwrap();

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
