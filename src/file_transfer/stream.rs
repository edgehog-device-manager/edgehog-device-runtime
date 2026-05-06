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

//! Protocol definitions for Pipe Data transmission.
//!
//! This module contains the data structures used for routing and status reporting
//! across the pipe. These structures are serialized using CBOR (via `minicbor`).
//!
//! # Stream Encoding
//!
//! Data written to and read from the pipe follows a strict, three-part sequential layout.
//! To facilitate safe boundary parsing over arbitrary streams, the header and footer
//! are encoded as length-prefixed CBOR payloads.
//!
//! The structure of a single data transmission across the pipe is:
//!
//! 1. **Header**: Length-prefixed CBOR encoding of `PipeDataHeader`.
//! 2. **Body**: Raw, unencoded bytes. The exact size of this payload corresponds to
//!    the `length` field specified within the decoded `PipeDataHeader`.
//! 3. **Footer**: Length-prefixed CBOR encoding of `PipeDataFooter`.
//!
//! # CDDL Specification
//!
//! Because the structs use positional tags (`#[cbor(n(x))]`), `minicbor` serializes
//! them as ordered CBOR arrays rather than maps.
//!
//! ```cddl
//! ; PipeDataHeader encodes as a single-element array containing an unsigned integer
//! PipeDataHeader = [
//!     length: uint
//! ]
//!
//! ; PipeDataFooter encodes as a single-element array containing a signed integer
//! PipeDataFooter = [
//!     status: int32
//! ]
//!
//! ; Explicit boundary definition for the 32-bit signed integer
//! int32 = -2147483648..2147483647
//! ```

use std::{io, task::Poll};

use bytes::{Bytes, BytesMut};
use eyre::Context;
use minicbor::CborLen;
use pin_project::pin_project;
use tokio::{
    io::{AsyncReadExt, AsyncWrite, ReadBuf, Take},
    task::JoinHandle,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::instrument;

use crate::{
    file_transfer::errno::{self},
    io::{digest::Digest, limit::Limit},
};

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub(crate) struct PipeDataHeader {
    #[cbor(n(0))]
    length: u64,
}

impl PipeDataHeader {
    fn new(length: u64) -> Self {
        Self { length }
    }
}

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub(crate) struct PipeDataFooter {
    #[cbor(n(0))]
    status: i32,
}

impl PipeDataFooter {
    fn new(status: i32) -> Self {
        Self { status }
    }

    pub(crate) fn check_status(&self) -> io::Result<()> {
        if self.status != errno::OK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "expected stream footer status to equal 0, got {}",
                    self.status
                ),
            ));
        }

        Ok(())
    }
}

pub(crate) struct MissingMetadata;

pub(crate) struct ReadBody(u64);

#[pin_project]
pub(crate) struct PipeStreamReader<R, S> {
    #[pin]
    inner: R,
    step: S,
}

impl<R> PipeStreamReader<R, MissingMetadata> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            step: MissingMetadata,
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn read_header(self) -> io::Result<PipeStreamReader<Take<R>, ReadBody>>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut inner = self.inner.compat();

        let buffer = vec![0u8; 32];
        let mut header_reader = minicbor_io::AsyncReader::with_buffer(&mut inner, buffer);

        let header: PipeDataHeader = header_reader
            .read()
            .await
            .map_err(io::Error::other)?
            .ok_or(io::Error::new(
                io::ErrorKind::InvalidData,
                "no header found while reading stream",
            ))?;

        let take = inner.into_inner().take(header.length);

        Ok(PipeStreamReader {
            inner: take,
            step: ReadBody(header.length),
        })
    }
}

impl<R> PipeStreamReader<Take<R>, ReadBody> {
    pub(crate) fn body_len(&self) -> u64 {
        self.step.0
    }

    #[instrument(skip_all)]
    pub(crate) async fn expect_footer(self) -> io::Result<PipeDataFooter>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut inner = self.inner.into_inner().compat();

        let buffer = vec![0u8; 16];
        let mut header_reader = minicbor_io::AsyncReader::with_buffer(&mut inner, buffer);

        header_reader
            .read()
            .await
            .map_err(io::Error::other)?
            .ok_or(io::Error::new(
                io::ErrorKind::InvalidData,
                "no footer found while reading stream",
            ))
    }
}

impl<R> tokio::io::AsyncRead for PipeStreamReader<Take<R>, ReadBody>
where
    R: tokio::io::AsyncRead,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.project().inner.poll_read(cx, buf)
    }
}

pub(crate) struct WriteMetadata {
    size: u64,
}

pub(crate) struct WriteBody;

#[pin_project]
pub(crate) struct PipeStreamWriter<W, S> {
    #[pin]
    inner: W,
    step: S,
}

impl<W> PipeStreamWriter<W, WriteMetadata> {
    #[instrument(skip_all)]
    pub(crate) fn new(inner: W, size: u64) -> Self {
        Self {
            inner,
            step: WriteMetadata { size },
        }
    }
}

impl<W> PipeStreamWriter<W, WriteMetadata>
where
    W: AsyncWrite + Unpin,
{
    #[instrument(skip_all)]
    pub(crate) async fn write_header(self) -> io::Result<PipeStreamWriter<Limit<W>, WriteBody>> {
        let metadata = PipeDataHeader::new(self.step.size);

        let inner = self.inner.compat_write();
        let buffer = vec![0u8; 4 + metadata.cbor_len(&mut ())];
        let mut writer = minicbor_io::AsyncWriter::with_buffer(inner, buffer);

        writer.write(metadata).await.map_err(io::Error::other)?;

        let inner = writer.into_parts().0.into_inner();

        Ok(PipeStreamWriter {
            inner: Limit::new(inner, self.step.size),
            step: WriteBody,
        })
    }
}

// impl<W> PipeStreamWriter<WriteBody<W>> {}

impl<W> tokio::io::AsyncWrite for PipeStreamWriter<Limit<W>, WriteBody>
where
    W: tokio::io::AsyncWrite,
{
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();

        this.inner.poll_write(cx, buf)
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

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.project();

        this.inner.poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

impl<W> Digest<PipeStreamWriter<Limit<W>, WriteBody>>
where
    W: AsyncWrite + Unpin,
{
    #[instrument(skip(self))]
    pub(crate) async fn write_footer(self, digest: &[u8]) -> io::Result<()> {
        let digest = self.check_digest(digest);

        let (inner, status) = match digest {
            Ok(w) => (w, errno::OK),
            Err(w) => (w, errno::to_errno(io::ErrorKind::InvalidData)),
        };

        let footer = PipeDataFooter::new(status);
        let buf = vec![0u8; 4 + footer.cbor_len(&mut ())];
        let mut writer =
            minicbor_io::AsyncWriter::with_buffer(inner.inner.into_inner().compat_write(), buf);

        writer.write(footer).await.map_err(io::Error::other)?;

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct SharedReaderStream<R> {
    handle: JoinHandle<eyre::Result<R>>,
}

impl<R> SharedReaderStream<R> {
    const CAPACITY: usize = 8 * 1024;

    #[instrument(skip_all)]
    pub(crate) fn spawn(inner: R) -> (Self, async_channel::Receiver<io::Result<Bytes>>)
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
    {
        let (tx, rx) = async_channel::bounded(1);

        let handle = tokio::task::spawn(async move {
            let mut reader = inner;
            let mut buf = BytesMut::with_capacity(Self::CAPACITY);

            loop {
                if buf.capacity() == 0 {
                    buf.reserve(Self::CAPACITY);
                }

                match reader.read_buf(&mut buf).await {
                    Ok(0) => break,
                    Ok(_n) => tx.send(Ok(buf.split().freeze())).await?,
                    Err(e) => {
                        tx.send(Err(e)).await?;
                        break;
                    }
                };
            }

            Ok(reader)
        });

        (Self { handle }, rx)
    }

    #[instrument(skip_all)]
    pub(crate) async fn join_reader(self) -> eyre::Result<R> {
        self.handle
            .await
            .wrap_err("join error while waiting for the reader")?
    }
}

// TODO create test to serialize and deserialize with insta
