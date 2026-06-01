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

use std::io;
use std::task::Poll;

use async_compression::tokio::bufread::GzipDecoder;
use async_compression::tokio::write::GzipEncoder;
use async_tar::{Archive, Entries};
use futures::StreamExt;
use pin_project::pin_project;
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio_stream::Stream;
use tokio_util::io::simplex;
use tracing::instrument;

use crate::file_transfer::encoding::{EncoderBuilder, TarEncoder};

#[derive(Clone, Debug)]
pub(crate) struct TarGzEncoding;

impl TarGzEncoding {
    fn encoder<W>(writer: W) -> TarEncoder<GzipEncoder<W>>
    where
        W: AsyncWrite + Unpin + Send + Sync,
    {
        let encoder = GzipEncoder::new(writer);

        TarEncoder::new(encoder)
    }
}

impl EncoderBuilder for TarGzEncoding {
    type Encoder = GzipEncoder<simplex::Sender>;

    fn build(self, writer: simplex::Sender) -> TarEncoder<Self::Encoder> {
        Self::encoder(writer)
    }
}

#[pin_project]
pub(crate) struct TarGzDecoder<R>
where
    R: AsyncBufRead + Unpin,
{
    archive: Entries<GzipDecoder<R>>,
}

impl<R> TarGzDecoder<R>
where
    R: AsyncBufRead + Unpin,
{
    #[instrument(skip_all)]
    pub(crate) fn create(reader: R) -> io::Result<Self> {
        Archive::new(GzipDecoder::new(reader))
            .entries()
            .map(|archive| Self { archive })
    }
}

impl<R> Stream for TarGzDecoder<R>
where
    R: AsyncBufRead + Unpin,
{
    type Item = io::Result<async_tar::Entry<Archive<GzipDecoder<R>>>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.project();

        this.archive.poll_next_unpin(cx)
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

        let mut writer = TarGzEncoding::encoder(file);

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

        let mut reader = TarGzDecoder::create(BufReader::new(file)).unwrap();

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
