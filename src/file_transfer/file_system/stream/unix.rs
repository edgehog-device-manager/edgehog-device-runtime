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

use std::path::PathBuf;

use eyre::Context;
use rustix::fs::Mode;
use tracing::{info, instrument, trace};
use uuid::Uuid;

use crate::file_transfer::FileOptions;

use super::Pipe;

#[derive(Debug, Clone)]
pub(crate) struct MakeFifo {
    dir: PathBuf,
}

impl MakeFifo {
    #[instrument(skip(self))]
    async fn pipe_path(&self, id: &Uuid) -> eyre::Result<PathBuf> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .wrap_err("couldn't create streaming directory")?;

        Ok(self.dir.join(id.to_string()))
    }
}

impl Pipe for MakeFifo {
    type Writer = tokio::net::unix::pipe::Sender;
    type Reader = tokio::net::unix::pipe::Receiver;

    fn new() -> Self {
        let mut dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);

        dir.push("edgehog-device-runtime");
        dir.push("streams");

        Self { dir }
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    async fn open_writer(&self, opt: &FileOptions) -> eyre::Result<Self::Writer> {
        let path = self.pipe_path(&opt.id).await?;

        trace!(fifo = %path.display(), "opening fifo");

        // TODO: try multiple times to open the fifo before giving up, since the reader could not
        //       have had the time to open the pipe
        let tx = tokio::net::unix::pipe::OpenOptions::new()
            .open_sender(&path)
            .wrap_err("couldn't open sender")?;

        info!("fifo opened");

        Ok(tx)
    }

    #[instrument(skip(self))]
    async fn create_reader(&self, id: &Uuid) -> eyre::Result<Self::Reader> {
        let path = self.pipe_path(id).await?;

        let dir = tokio::fs::File::open(&self.dir)
            .await
            .wrap_err("failed to open directory")?
            .into_std()
            .await;

        trace!(fifo = %path.display(), "opening fifo");

        tokio::task::spawn_blocking({
            let path = path.clone();

            move || rustix::fs::mkfifoat(dir, path, Mode::RUSR | Mode::WUSR)
        })
        .await?
        .wrap_err("couldn't make fifo")?;

        let rx = tokio::net::unix::pipe::OpenOptions::new()
            .unchecked(true)
            .open_receiver(&path)
            .wrap_err("couldn't open sender")?;

        info!("fifo opened");

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempdir::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;

    use crate::file_transfer::FileOptions;
    #[cfg(unix)]
    use crate::file_transfer::request::FilePermissions;

    use super::*;

    fn mkf() -> (MakeFifo, TempDir) {
        let dir = TempDir::new("mk_fifo").unwrap();

        let mk = MakeFifo {
            dir: dir.path().to_path_buf(),
        };

        (mk, dir)
    }

    #[test]
    fn runtime() {
        MakeFifo::new();
    }

    #[tokio::test]
    async fn pipe_path() {
        let (mut mk, dir) = mkf();

        let pipe_dir = dir.path().join("streaming");

        mk.dir = pipe_dir.clone();

        let id = Uuid::new_v4();

        mk.pipe_path(&id).await.unwrap();

        let meta = tokio::fs::metadata(pipe_dir).await.unwrap();

        assert!(meta.is_dir());
    }

    #[tokio::test]
    async fn make_pipe() {
        let (mk, _dir) = mkf();

        let exp = b"Hello, World!";

        let opt = FileOptions {
            id: Uuid::new_v4(),
            file_size: exp.len().try_into().unwrap(),
            #[cfg(unix)]
            perm: FilePermissions {
                mode: None,
                user_id: None,
                group_id: None,
            },
            compression: None,
        };

        let rx = mk.create_reader(&opt.id).await.unwrap();
        let mut tx = mk.open_writer(&opt).await.unwrap();

        let mut buf = vec![0; exp.len()];

        let written = tx.write(exp).await.unwrap();
        assert_eq!(written, exp.len());

        let mut take = rx.take(opt.file_size);

        take.read_exact(&mut buf).await.unwrap();

        assert_eq!(buf, exp);
    }
}
