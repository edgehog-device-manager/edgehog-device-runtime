// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::io;

use pin_project::pin_project;
use tokio::io::AsyncWrite;
use tracing::instrument;

#[derive(Debug)]
#[pin_project]
pub(crate) struct Limit<T> {
    remaining: u64,
    #[pin]
    inner: T,
}

impl<T> Limit<T> {
    pub(crate) fn new(limit: u64, inner: T) -> Self {
        Self {
            remaining: limit,
            inner,
        }
    }
}

impl<W> AsyncWrite for Limit<W>
where
    W: AsyncWrite,
{
    #[instrument(skip_all, ret)]
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let buf_len =
            u64::try_from(buf.len()).map_err(|e| io::Error::new(io::ErrorKind::FileTooLarge, e))?;

        if self.remaining < buf_len {
            return std::task::Poll::Ready(Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "write exceeds file limit",
            )));
        }

        let this = self.project();

        let written = std::task::ready!(this.inner.poll_write(cx, buf))?;

        // NOTE it must be guaranteed that `written <= buf.len()`
        debug_assert!(written <= buf.len());
        *this.remaining -= written as u64;

        std::task::Poll::Ready(Ok(written))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.project();

        this.inner.poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.project();

        this.inner.poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    #[tokio::test]
    #[should_panic]
    async fn test_limit() {
        let mut buf = Vec::<u8>::new();
        let mut progress = Limit::new(1 << 5, &mut buf);

        let expected = [0xFF; 1 << 6];

        progress.write_all(&expected).await.unwrap();
    }

    #[tokio::test]
    async fn test_write() {
        let mut buf = Vec::<u8>::new();
        let mut progress = Limit::new(1 << 5, &mut buf);

        let expected = [0xFF; 1 << 5];

        progress.write_all(&expected).await.unwrap();

        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn test_double_write() {
        let mut buf = Vec::<u8>::new();
        let mut progress = Limit::new(1 << 5, &mut buf);

        let expected = [0xFF; 1 << 5];

        let res = progress.write_all(&expected).await;
        assert!(res.is_ok());

        let res = progress.write_all(&expected[..1]).await;
        assert!(res.is_err());
    }
}
