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
use std::path::{Path, PathBuf};

use pin_project::pin_project;
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, instrument, trace};
use uuid::Uuid;

use crate::file_transfer::encoding::tar_gz::TarGzDecoder;
use crate::io::digest::Digest;

#[cfg(unix)]
use super::request::FilePermissions;
use super::request::{Encoding, FileDigest};

pub(super) mod store;
pub(super) mod stream;
pub(crate) mod walk;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileOptions {
    pub(super) id: Uuid,
    pub(super) file_size: u64,
    #[cfg(unix)]
    pub(super) perm: FilePermissions,
    pub(super) compression: Option<Encoding>,
}

#[derive(Debug)]
#[pin_project]
pub(crate) struct WriteHandle {
    /// Path to the file
    path: PathBuf,
    /// Path to the file with `.part` extension.
    partial: PathBuf,
    /// Starting size of the file.
    current_size: u64,
    /// Opened file.
    #[pin]
    file: tokio::fs::File,
}

impl WriteHandle {
    const PARTTIAL_EXT: &str = ".part";

    pub(crate) fn current_size(&self) -> u64 {
        self.current_size
    }

    #[cfg(unix)]
    fn mask(mode: u32) -> u32 {
        0o600 | mode
    }

    #[instrument]
    pub(crate) async fn try_exists(
        path: &Path,
        alg: FileDigest,
        digest: &[u8],
    ) -> io::Result<bool> {
        let file = match File::open(path).await {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => {
                error!(%error, "couldn't check if file existed");

                return Err(error);
            }
        };

        let meta = file.metadata().await?;

        let file = Digest::from_read(file, alg, meta.len()).await?;

        file.check_digest(digest)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "data digest mismatch"))?;

        Ok(true)
    }

    #[instrument(skip(opt))]
    pub(crate) async fn open(path: PathBuf, opt: &FileOptions) -> io::Result<Self> {
        // Set partial extension
        let mut partial = path.clone().into_os_string();
        partial.push(Self::PARTTIAL_EXT);
        let partial = PathBuf::from(partial);

        let mut file_options = File::options();

        file_options.create(true).write(true).read(true);

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                file_options.mode(Self::mask(opt.perm.mode()));
            } else {
                let _opt = opt;
            }
        }

        // TODO: flock the file?
        let file = file_options.open(&partial).await?;

        let current_size = file.metadata().await?.len();

        trace!(current_size, "reading existing content");

        trace!("returning the handle");

        Ok(WriteHandle {
            path,
            partial,
            current_size,
            file,
        })
    }

    #[instrument(skip_all, fields(self.path))]
    pub(crate) async fn finalize(&mut self, opt: &FileOptions) -> io::Result<()> {
        self.file.flush().await?;
        self.file.sync_all().await?;

        if let Some(comp) = opt.compression {
            self.extract(comp).await
        } else {
            self.move_part(opt).await
        }
    }

    #[instrument(skip_all)]
    async fn move_part(&mut self, opt: &FileOptions) -> io::Result<()> {
        let from = &self.partial;
        let to = &self.path;

        tokio::fs::rename(from, to)
            .await
            .inspect_err(|error| error!(%error, "couldn't rename the file"))?;

        info!("file stored");

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                let gid = opt.perm.group_id;
                let to = to.clone();

                tokio::task::spawn_blocking(move || {
                    if let Err(error) = std::os::unix::fs::chown(to, None, gid) {
                        use tracing::error;

                        error!(%error, "couldn't change file ownership, this could be because the user is unprivileged and doesn't belongs to the target group");
                    }
                })
                .await?;
            } else {
                let _ = opt;
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn extract(&mut self, compression: Encoding) -> io::Result<()> {
        self.file.seek(io::SeekFrom::Start(0)).await?;

        tokio::fs::create_dir_all(&self.path).await?;

        match compression {
            Encoding::TarGz => {
                let mut extract = TarGzDecoder::create(BufReader::new(&mut self.file))?;

                while let Some(item) = extract.next().await {
                    let mut item = item?;

                    item.unpack_in(&self.path).await?;
                }
            }
        }

        info!("file extracted");

        tokio::fs::remove_file(&self.partial).await?;

        Ok(())
    }

    #[instrument(skip_all, fields(self.path))]
    pub(crate) async fn cleanup(self) -> io::Result<()> {
        debug!("closing file");
        drop(self.file);
        Self::cleanup_partial(&self.partial).await
    }

    async fn cleanup_partial(partial: &Path) -> io::Result<()> {
        debug!("removing file");

        match tokio::fs::remove_file(partial).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                debug!("file already missing");

                Ok(())
            }
            Err(error) => {
                error!(%error, "couldn't delete the file");

                Err(error)
            }
        }
    }
}

impl AsyncWrite for WriteHandle {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();

        this.file.poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.project();

        this.file.poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.project();

        this.file.poll_shutdown(cx)
    }
}

impl AsyncRead for WriteHandle {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.project();

        this.file.poll_read(cx, buf)
    }
}

impl AsyncSeek for WriteHandle {
    fn start_seek(
        self: std::pin::Pin<&mut Self>,
        position: std::io::SeekFrom,
    ) -> std::io::Result<()> {
        let this = self.project();

        this.file.start_seek(position)
    }

    fn poll_complete(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<u64>> {
        let this = self.project();

        this.file.poll_complete(cx)
    }
}

#[cfg(test)]
mod tests {
    use tempdir::TempDir;

    use super::*;

    #[tokio::test]
    async fn try_exists() {
        let dir = TempDir::new("try_exists").unwrap();

        let content = Uuid::new_v4().to_string();

        let alg = FileDigest::Sha256;
        let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, content.as_bytes());

        let path = dir.path().join("file.txt");

        tokio::fs::write(&path, &content).await.unwrap();

        let exists = WriteHandle::try_exists(&path, alg, digest.as_ref())
            .await
            .unwrap();

        assert!(exists);
    }

    #[tokio::test]
    async fn mismatched_digest() {
        let dir = TempDir::new("try_exists").unwrap();

        let content = Uuid::new_v4().to_string();

        let alg = FileDigest::Sha256;
        let other_content = Uuid::new_v4().to_string();
        let digest =
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, other_content.as_bytes());

        let path = dir.path().join("file.txt");

        tokio::fs::write(&path, &content).await.unwrap();

        WriteHandle::try_exists(&path, alg, digest.as_ref())
            .await
            .unwrap_err();
    }
}
