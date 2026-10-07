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

use eyre::Context;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tracing::{info, instrument, trace};
use uuid::Uuid;

use std::path::PathBuf;

use crate::file_transfer::FileOptions;

use super::Pipe;

#[derive(Debug, Clone)]
pub(crate) struct MakeNamedPipe {}

impl MakeNamedPipe {
    fn pipe_name(&self, id: &Uuid) -> PathBuf {
        PathBuf::from(format!(r"\\.\pipe\edgehog-device-runtime-stream-{id}"))
    }
}

impl Pipe for MakeNamedPipe {
    type Writer = NamedPipeServer;
    type Reader = NamedPipeServer;

    fn new() -> Self {
        Self {}
    }

    fn get_path(&self, id: &Uuid) -> PathBuf {
        self.pipe_name(id)
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    async fn open_writer(&self, opt: &FileOptions) -> eyre::Result<Self::Writer> {
        let pipe_name = self.pipe_name(&opt.id);

        trace!(pipe_name = %pipe_name.display(), "opening server to write");

        let tx = ServerOptions::new()
            .access_outbound(false)
            .first_pipe_instance(true)
            .create(pipe_name)
            .wrap_err("couldn't create server")?;

        info!("named pipe opened");

        Ok(tx)
    }

    #[instrument(skip(self))]
    async fn create_reader(&self, id: &Uuid) -> eyre::Result<Self::Reader> {
        let pipe_name = self.pipe_name(id);

        trace!(pipe_name = %pipe_name.display(), "opening server to read");

        let rx = ServerOptions::new()
            .access_outbound(false)
            .first_pipe_instance(true)
            .create(pipe_name)
            .wrap_err("couldn't create server")?;

        rx.connect().await.wrap_err("couldn't wait client")?;

        info!("named pipe opened");

        Ok(rx)
    }

    #[instrument(skip_all)]
    async fn remove_reader(&self, _id: &Uuid) -> eyre::Result<()> {
        // NOTE in windows dropping the pipe is sufficient
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;

    use crate::file_transfer::FileOptions;

    use super::*;

    fn mkp() -> MakeNamedPipe {
        MakeNamedPipe {}
    }

    #[tokio::test]
    async fn pipe_name() {
        let mk = mkp();

        let id = Uuid::new_v4();

        let exp = format!(r"\\.\pipe\edgehog-device-runtime-stream-{id}");

        assert_eq!(mk.pipe_name(&id), exp);
    }

    #[rstest::rstest]
    #[timeout(std::time::Duration::from_secs(5))]
    #[tokio::test]
    async fn make_pipe() {
        let mk = mkp();

        let exp = b"Hello, World!";

        let opt = FileOptions {
            id: Uuid::new_v4(),
            file_size: exp.len().try_into().unwrap(),
            compression: None,
        };

        let read = mk.create_reader(&opt.id);
        let write = mk.open_writer(&opt);

        let (rx, mut tx) = tokio::try_join!(read, write).unwrap();

        let mut buf = vec![0; exp.len()];
        let mut take = rx.take(opt.file_size);

        let write = tx.write(exp);
        let read = take.read(&mut buf);

        let (read, written) = tokio::try_join!(write, read).unwrap();

        assert_eq!(written, exp.len());
        assert_eq!(read, exp.len());

        assert_eq!(buf, exp);
    }
}
