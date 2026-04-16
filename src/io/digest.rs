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

use std::fmt::Debug;
use std::io::{self};
use std::task::ready;

use pin_project::pin_project;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, BufReader};
use tracing::instrument;

use crate::file_transfer::request::FileDigest;

/// Wrapper to a [`AsyncRead`] or [`AsyncWrite`] that calculates the digest.
#[pin_project]
pub(crate) struct Digest<S> {
    ctx: aws_lc_rs::digest::Context,
    #[pin]
    inner: S,
}

impl<S> Digest<S> {
    pub(crate) fn new(inner: S, digest: FileDigest) -> Self {
        Self {
            ctx: digest.into(),
            inner,
        }
    }

    #[instrument(skip(inner))]
    pub(crate) async fn from_read(inner: S, digest: FileDigest, seek: u64) -> io::Result<Self>
    where
        S: AsyncRead + Unpin,
    {
        let mut ctx = aws_lc_rs::digest::Context::from(digest);

        let inner = if seek == 0 {
            inner
        } else {
            // Read till start
            let mut reader = BufReader::new(inner.take(seek));

            while let buf = reader.fill_buf().await?
                && !buf.is_empty()
            {
                ctx.update(buf);

                let len = buf.len();

                reader.consume(len);
            }

            reader.into_inner().into_inner()
        };

        Ok(Self { ctx, inner })
    }

    #[instrument(skip_all)]
    pub(crate) fn check_digest(self, digest: &[u8]) -> Result<S, S> {
        if self.ctx.finish().as_ref() != digest {
            return Err(self.inner);
        }

        Ok(self.inner)
    }
}

impl<S> Debug for Digest<S>
where
    S: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { ctx: _, inner } = self;

        f.debug_struct("Digest")
            .field("inner", &inner)
            .finish_non_exhaustive()
    }
}

impl<S> AsyncWrite for Digest<S>
where
    S: AsyncWrite,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.project();

        this.ctx.update(buf);

        this.inner.poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.project();

        this.inner.poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.project();

        this.inner.poll_shutdown(cx)
    }
}

impl<S> AsyncRead for Digest<S>
where
    S: AsyncRead,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.project();

        let current = buf.filled().len();

        ready!(this.inner.poll_read(cx, buf))?;

        let read = buf
            .filled()
            .get(current..)
            .ok_or(io::Error::from(io::ErrorKind::UnexpectedEof))?;

        this.ctx.update(read);

        std::task::Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tokio::fs::File;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn read() {
        let dir = tempdir::TempDir::new("read").unwrap();

        let content = "content to be hashed";
        let path = dir.path().join("file.txt");

        tokio::fs::write(&path, content).await.unwrap();

        let file = File::open(&path).await.unwrap();
        let mut reader = Digest::new(file, FileDigest::Sha256);

        let mut buf = String::new();
        reader.read_to_string(&mut buf).await.unwrap();

        assert_eq!(buf, content);

        let exp = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, content.as_bytes());

        let _ = reader.check_digest(exp.as_ref()).unwrap();
    }

    #[tokio::test]
    async fn write() {
        let dir = tempdir::TempDir::new("write").unwrap();

        let content = "content to be hashed";
        let path = dir.path().join("file.txt");

        let file = File::options()
            .create_new(true)
            .write(true)
            .open(&path)
            .await
            .unwrap();
        let mut writer = Digest::new(file, FileDigest::Sha256);

        writer.write_all(content.as_bytes()).await.unwrap();

        writer.flush().await.unwrap();

        let buf = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(buf, content);

        let exp = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, content.as_bytes());

        let _ = writer.check_digest(exp.as_ref()).unwrap();
    }

    #[tokio::test]
    async fn existing() {
        let dir = tempdir::TempDir::new("write").unwrap();

        let content = "content to be hashed";
        let path = dir.path().join("file.txt");
        let idx = 5;

        tokio::fs::write(&path, &content.as_bytes()[..idx])
            .await
            .unwrap();

        let file = File::options()
            .write(true)
            .read(true)
            .open(&path)
            .await
            .unwrap();

        let mut writer = Digest::from_read(file, FileDigest::Sha256, idx.try_into().unwrap())
            .await
            .unwrap();

        writer.write_all(&content.as_bytes()[idx..]).await.unwrap();

        writer.flush().await.unwrap();

        let buf = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(buf, content);

        let exp = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, content.as_bytes());

        let _ = writer.check_digest(exp.as_ref()).unwrap();
    }
}
