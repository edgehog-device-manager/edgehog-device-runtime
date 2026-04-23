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

use std::io;

use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite};

// NOTE currently this could be used to both read and write but it would increment a single counter decide whether
// it makes more sense to create two new types wrapper to only allow read or write, the same applies to the `Limit`
#[derive(Debug)]
#[pin_project]
pub(crate) struct Progress<T, F> {
    #[pin]
    inner: T,
    total: u64,
    // To be generic over the content of the watch provide a function that updates the value
    progress: F,
}

impl<T, F> Progress<T, F> {
    pub(crate) fn new(inner: T, progress: F) -> Self {
        Self {
            inner,
            total: 0,
            progress,
        }
    }

    pub(crate) fn into_inner(self) -> T {
        self.inner
    }

    pub(crate) fn into_total(self) -> u64 {
        self.total
    }
}

impl<W, F> AsyncWrite for Progress<W, F>
where
    W: AsyncWrite,
    F: ProgressHandle,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let mut this = self.project();

        let written = std::task::ready!(this.inner.as_mut().poll_write(cx, buf))?;

        *this.total = u64::try_from(written)
            .ok()
            .and_then(|w| this.total.checked_add(w))
            .ok_or(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "file written exceeds supported size",
            ))?;

        this.progress.update(*this.total)?;

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

impl<R, F> AsyncRead for Progress<R, F>
where
    R: AsyncRead,
    F: ProgressHandle,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let mut this = self.project();

        let before = buf.filled().len();

        std::task::ready!(this.inner.as_mut().poll_read(cx, buf))?;

        *this.total = u64::try_from(buf.filled().len() - before)
            .ok()
            .and_then(|r| this.total.checked_add(r))
            .ok_or(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "file read exceeds supported size",
            ))?;

        this.progress.update(*this.total)?;

        std::task::Poll::Ready(Ok(()))
    }
}

/// Handle the progress of an IO operation.
#[cfg_attr(test, mockall::automock)]
pub(crate) trait ProgressHandle {
    /// Callback with the progress in bytes.
    ///
    /// This is the total number of bytes read or written.
    fn update(&mut self, bytes: u64) -> io::Result<()>;
}

/// If you just wish to use the progress to track the total of bytes written you can use the unit tuple
impl ProgressHandle for () {
    fn update(&mut self, _bytes: u64) -> io::Result<()> {
        // do nothing
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::array;

    use mockall::{
        Sequence,
        predicate::{eq, le},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn test_write() {
        let mut seq = Sequence::new();
        let mut mock_prog_handle = MockProgressHandle::new();

        mock_prog_handle
            .expect_update()
            .with(eq(5))
            .in_sequence(&mut seq)
            .return_once(|_| Ok(()));

        mock_prog_handle
            .expect_update()
            .with(eq(10))
            .in_sequence(&mut seq)
            .return_once(|_| Ok(()));

        let mut buf = Vec::<u8>::new();
        let mut progress = Progress::new(&mut buf, mock_prog_handle);

        let expected = [0xFF; 10];

        progress.write_all(&expected[..5]).await.unwrap();
        progress.write_all(&expected[5..]).await.unwrap();

        assert_eq!(buf, expected);
    }

    #[tokio::test]
    async fn test_read() {
        let mut seq = Sequence::new();
        let mut mock_prog_handle = MockProgressHandle::new();

        mock_prog_handle
            .expect_update()
            .with(le(1 << 10))
            .in_sequence(&mut seq)
            .returning(|_| Ok(()));

        const SPLIT: usize = 1 << 9;

        let expected: [u8; 1 << 10] = array::from_fn(|i| (i % 42) as u8);

        let mut progress = Progress::new(expected.as_slice(), mock_prog_handle);

        let mut out = [0u8; SPLIT];

        progress.read_exact(&mut out).await.unwrap();
        assert_eq!(&out, &expected[..SPLIT]);

        progress.read_exact(&mut out).await.unwrap();
        assert_eq!(&out, &expected[SPLIT..]);
    }
}
