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
use futures::Stream;
use minicbor::CborLen;
use pin_project::pin_project;
use tokio::{
    io::{AsyncReadExt, AsyncWrite, ReadBuf, Take},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, instrument};

use crate::{file_transfer::errno, io::limit::Limit};

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub struct PipeDataHeader {
    #[cbor(n(0))]
    length: u64,
}

impl PipeDataHeader {
    fn new(length: u64) -> Self {
        Self { length }
    }
}

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode, minicbor::CborLen)]
pub struct PipeDataFooter {
    #[cbor(n(0))]
    status: i32,
}

impl PipeDataFooter {
    fn new(status: i32) -> Self {
        Self { status }
    }

    pub fn check_status(&self) -> io::Result<()> {
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

pub struct MissingMetadata;

pub struct ReadBody(u64);

#[pin_project]
pub struct PipeStreamReader<R, S> {
    #[pin]
    inner: R,
    step: S,
}

impl<R> PipeStreamReader<R, MissingMetadata> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            step: MissingMetadata,
        }
    }

    #[instrument(skip_all)]
    pub async fn read_header(self) -> io::Result<PipeStreamReader<Take<R>, ReadBody>>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let mut inner = self.inner.compat();

        let buffer = vec![0u8; 32];
        let mut header_reader = minicbor_io::AsyncReader::with_buffer(&mut inner, buffer);

        let header: PipeDataHeader = header_reader
            .read()
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid header field"))?
            .ok_or(io::Error::new(
                io::ErrorKind::InvalidData,
                "no header found while reading stream",
            ))?;

        debug!(length = header.length, "received length");

        let take = inner.into_inner().take(header.length);

        Ok(PipeStreamReader {
            inner: take,
            step: ReadBody(header.length),
        })
    }
}

impl<R> PipeStreamReader<Take<R>, ReadBody> {
    pub fn body_len(&self) -> u64 {
        self.step.0
    }

    #[instrument(skip_all)]
    pub async fn expect_footer(self) -> io::Result<PipeDataFooter>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        if self.inner.limit() > 0 {
            return Err(io::Error::other(
                "reader limit has to be reached to expect footer",
            ));
        }

        let mut inner = self.inner.into_inner().compat();

        let buffer = vec![0u8; 16];
        let mut header_reader = minicbor_io::AsyncReader::with_buffer(&mut inner, buffer);

        header_reader
            .read()
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid footer field"))?
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

pub struct WriteMetadata {
    size: u64,
}

pub struct WriteBody;

#[pin_project]
pub struct PipeStreamWriter<W, S> {
    #[pin]
    inner: W,
    step: S,
}

impl<W> PipeStreamWriter<W, WriteMetadata> {
    #[instrument(skip_all)]
    pub fn new(inner: W, size: u64) -> Self {
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
    pub async fn write_header(self) -> io::Result<PipeStreamWriter<Limit<W>, WriteBody>> {
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

impl<W> tokio::io::AsyncWrite for PipeStreamWriter<W, WriteBody>
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

impl<W> PipeStreamWriter<Limit<W>, WriteBody>
where
    W: AsyncWrite + Unpin,
{
    #[instrument(skip(self))]
    pub async fn write_footer(self, status: i32) -> io::Result<()> {
        let footer = PipeDataFooter::new(status);
        let buf = vec![0u8; 4 + footer.cbor_len(&mut ())];
        let mut writer =
            minicbor_io::AsyncWriter::with_buffer(self.inner.into_inner().compat_write(), buf);

        writer.write(footer).await.map_err(io::Error::other)?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct SharedReaderStream<R> {
    handle: JoinHandle<eyre::Result<R>>,
}

impl<R> SharedReaderStream<R> {
    const CAPACITY: usize = 8 * 1024;

    #[instrument(skip_all)]
    pub fn spawn(inner: R) -> (Self, impl Stream<Item = io::Result<Bytes>>)
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
    {
        let (tx, rx) = tokio::sync::mpsc::channel(1);

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

        (Self { handle }, ReceiverStream::new(rx))
    }

    #[instrument(skip_all)]
    pub async fn join_reader(self) -> eyre::Result<R> {
        self.handle
            .await
            .wrap_err("join error while waiting for the reader")?
    }
}

#[cfg(test)]
mod tests {
    use bytes::BufMut;
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    use crate::{
        file_transfer::request::FileDigest,
        io::digest::Digest,
        tests::{Hexdump, with_insta},
    };

    use super::*;

    fn get_digest(content: &[u8], digest: FileDigest) -> Vec<u8> {
        let mut ctx = aws_lc_rs::digest::Context::from(digest);
        ctx.update(content);
        ctx.finish().as_ref().to_owned()
    }

    async fn write_pipe(content: &[u8], buf: &mut Vec<u8>) {
        let content_digest = get_digest(content, FileDigest::Sha256);

        let writer = PipeStreamWriter::new(buf, content.len() as u64);
        let mut writer = Digest::new(writer.write_header().await.unwrap(), FileDigest::Sha256);

        writer.write_all(content).await.unwrap();

        let writer = writer
            .check_digest(&content_digest)
            .map_err(|_| ())
            .unwrap();

        writer.write_footer(errno::OK).await.unwrap();
    }

    #[tokio::test]
    async fn pipe_writer() {
        let mut buf = Vec::new();
        let write_content = b"ciaociao";

        write_pipe(write_content, &mut buf).await;

        with_insta!({
            insta::assert_snapshot!(Hexdump(buf));
        });
    }

    #[tokio::test]
    async fn pipe_round_trip() {
        let mut buf = Vec::new();
        let write_content = include_bytes!("./mod.rs");

        write_pipe(write_content, &mut buf).await;

        let mut reader = PipeStreamReader::new(buf.as_slice())
            .read_header()
            .await
            .unwrap();

        assert_eq!(reader.body_len(), write_content.len() as u64);

        let mut read_data = BytesMut::with_capacity(write_content.len());
        reader.read_buf(&mut read_data).await.unwrap();

        assert_eq!(write_content.as_slice(), read_data);

        let out = reader.expect_footer().await.unwrap();
        assert_eq!(out.status, errno::OK);
        out.check_status().unwrap();
    }

    #[tokio::test]
    async fn shared_reader() {
        let buf = b"test bytes to read";
        let take: usize = 10;

        let (shared, mut stream) = SharedReaderStream::spawn(buf.take(take as u64));

        let mut read_stream_buf = BytesMut::with_capacity(take);

        while let Some(chunk) = stream.next().await {
            read_stream_buf.put(chunk.unwrap());
        }

        assert_eq!(&buf[0..take], read_stream_buf);

        let reader = shared.join_reader().await.unwrap().into_inner();

        assert_eq!(reader, &buf[take..]);
    }
}
