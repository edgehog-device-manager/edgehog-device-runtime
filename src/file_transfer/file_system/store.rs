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

//! Handles file system operations for the file transfer.

use std::fmt::{Debug, Display};
use std::io;
use std::path::{Path, PathBuf};

use eyre::WrapErr;
use futures::StreamExt;
use pin_project::pin_project;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, BufReader};
use tracing::{info, instrument, trace};
use uuid::Uuid;

use crate::file_transfer::compression::tar_gz::TarGzReader;
use crate::file_transfer::file_system::walk::Walk;
use crate::file_transfer::request::Compression;

use super::FileOptions;

/// Stores files in the storage
#[derive(Debug)]
pub(crate) struct FileStorage<F> {
    dir: PathBuf,
    fs: F,
}

impl FileStorage<Fs> {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { fs: Fs {}, dir }
    }
}

impl<F> FileStorage<F> {
    #[instrument(skip_all)]
    pub(crate) async fn init(&self) -> eyre::Result<()> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .wrap_err("couldn't create file storage directory")?;

        Ok(())
    }

    fn file_path(&self, id: &Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    fn partial_file_path(&self, id: &Uuid) -> PathBuf {
        self.dir.join(format!("{id}.part"))
    }

    #[cfg(unix)]
    fn mask(mode: u32) -> u32 {
        0o600 | mode
    }

    #[instrument(skip(self), ret)]
    pub(crate) async fn file_exists(&self, id: &Uuid) -> eyre::Result<bool> {
        let path = self.file_path(id);

        tokio::fs::try_exists(&path)
            .await
            .wrap_err_with(|| format!("couldn't access file: {}", path.display()))
    }

    #[instrument(skip(self))]
    pub(crate) async fn walk(&self, id: &Uuid) -> Option<(Walk, PathBuf)> {
        let path = self.file_path(id);

        path.exists().then(|| {
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();

            (Walk::new(path), parent)
        })
    }

    #[instrument(skip(self))]
    pub(crate) async fn open_read(&self, id: &Uuid) -> eyre::Result<tokio::fs::File> {
        let path = self.file_path(id);

        trace!(path = %path.display(), "opening file for read");

        // TODO: flock the file?
        let file = tokio::fs::File::options().read(true).open(path).await?;

        Ok(file)
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    pub(crate) async fn create_write_handle(
        &self,
        opt: &FileOptions,
    ) -> eyre::Result<(WriteHandle, aws_lc_rs::digest::Context)>
    where
        F: Space,
    {
        let file_path = self.partial_file_path(&opt.id);

        self.fs
            .reserve_space(opt.id, &file_path, opt.file_size)
            .await?;

        trace!(path = %file_path.display(), "opening file for write");

        let mut file_options = File::options();

        file_options.create(true).write(true).read(true);

        #[cfg(unix)]
        file_options.mode(Self::mask(opt.perm.mode()));

        // TODO: flock the file?
        let file = file_options.open(&file_path).await?;

        let current_size = file.metadata().await?.len();

        trace!(current_size, "reading existing content");

        let mut reader = BufReader::new(file);
        let mut digest = aws_lc_rs::digest::Context::from(opt.file_digest);

        // Will seek to the end of the file
        while let buf = reader.fill_buf().await?
            && !buf.is_empty()
        {
            digest.update(buf);

            let len = buf.len();

            reader.consume(len);
        }

        trace!("returning the handle");

        Ok((
            WriteHandle {
                id: opt.id,
                current_size,
                file: reader.into_inner(),
            },
            digest,
        ))
    }

    // TODO: validate the digest
    #[instrument(skip_all, fields(id = %handle.id))]
    pub(crate) async fn finalize_write(
        &self,
        handle: WriteHandle,
        opt: &FileOptions,
    ) -> eyre::Result<()>
    where
        F: Space,
    {
        handle
            .file
            .sync_all()
            .await
            .wrap_err("couldn't fsync the file")?;

        if let Some(comp) = opt.compression {
            self.extract(handle, comp).await?;
        } else {
            self.move_part(handle, opt).await?;
        }

        self.fs.finalize(opt.id).await?;

        Ok(())
    }

    async fn move_part(&self, handle: WriteHandle, opt: &FileOptions) -> eyre::Result<()> {
        let from = self.partial_file_path(&handle.id);
        let to = self.file_path(&handle.id);

        tokio::fs::rename(&from, &to)
            .await
            .wrap_err("couldn't rename the file")?;

        info!("file stored");

        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                let gid = opt.perm.group_id;

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
    async fn extract(&self, mut handle: WriteHandle, compression: Compression) -> eyre::Result<()> {
        handle.seek(io::SeekFrom::Start(0)).await?;

        let out = self.file_path(&handle.id);
        tokio::fs::create_dir_all(&out)
            .await
            .wrap_err("couldn't create output directory")?;

        match compression {
            Compression::TarGz => {
                let mut extract = TarGzReader::create(BufReader::new(handle.file))?;

                while let Some(item) = extract.next().await {
                    let mut item = item?;

                    item.unpack_in(&out)
                        .await
                        .wrap_err("couldn't unpack entry")?;
                }
            }
        }

        info!("file extracted");

        tokio::fs::remove_file(self.partial_file_path(&handle.id))
            .await
            .wrap_err("couldn't remove the partial file")?;

        Ok(())
    }
}

#[derive(Debug)]
#[pin_project]
pub(crate) struct WriteHandle {
    id: Uuid,
    current_size: u64,
    // TODO limit the size of the file
    #[pin]
    file: tokio::fs::File,
}

impl WriteHandle {
    pub(crate) fn current_size(&self) -> u64 {
        self.current_size
    }
}

impl Display for WriteHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WriteHandle(id={})", self.id)
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

#[derive(Debug)]
#[pin_project]
pub(crate) struct Limit<W> {
    remaining: u64,
    #[pin]
    inner: W,
}

impl<W> Limit<W> {
    pub(crate) fn new(limit: u64, inner: W) -> Self {
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

#[cfg_attr(test, mockall::automock)]
pub(crate) trait Space {
    /// Reserves the space for the file on the device.
    ///
    /// It will make sure that at least the 10% of free space is available on the device the files
    /// are stored on.
    fn reserve_space(
        &self,
        id: Uuid,
        path: &Path,
        file_size: u64,
    ) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Marks the file as saved and refreshes the current quota
    fn finalize(&self, _id: Uuid) -> impl Future<Output = eyre::Result<()>> + Send;
}

#[derive(Debug)]
pub(crate) struct Fs {}

impl Fs {}

impl Space for Fs {
    async fn reserve_space(&self, _id: Uuid, _path: &Path, _file_size: u64) -> eyre::Result<()> {
        // TODO: ensure 10% of the free space on disk

        Ok(())
    }

    async fn finalize(&self, _id: Uuid) -> eyre::Result<()> {
        // TODO: refresh the space

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    use mockall::{Sequence, predicate};
    use tempdir::TempDir;
    use tokio::io::AsyncWriteExt;

    use crate::file_transfer::request::FileDigest;
    #[cfg(unix)]
    use crate::file_transfer::request::FilePermissions;

    use super::*;

    use pretty_assertions::assert_eq;

    fn fs_storage() -> (FileStorage<Fs>, TempDir) {
        let dir = TempDir::new("fs_storage").expect("couldn't create temp directory");

        (FileStorage::new(dir.path().to_path_buf()), dir)
    }

    fn mock_fs_storage(mock: MockSpace) -> (FileStorage<MockSpace>, TempDir) {
        let dir = TempDir::new("fs_storage").expect("couldn't create temp directory");

        (
            FileStorage {
                dir: dir.path().to_path_buf(),
                fs: mock,
            },
            dir,
        )
    }

    #[tokio::test]
    async fn should_get_file_handle() {
        let (store, dir) = fs_storage();

        let id = Uuid::new_v4();

        assert_eq!(store.file_path(&id), dir.path().join(id.to_string()));
    }

    #[tokio::test]
    async fn write_handle_mock() {
        let id = Uuid::new_v4();

        let content = "Hello world";
        let exp_digest =
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, "Hello world".as_bytes());

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
            file_digest: FileDigest::Sha256,
            #[cfg(unix)]
            perm: FilePermissions {
                mode: Some(0o600),
                user_id: None,
                group_id: Some(100),
            },
            compression: None,
        };

        let mut mock = MockSpace::new();
        let mut seq = Sequence::new();

        mock.expect_reserve_space()
            .with(
                predicate::eq(opt.id),
                predicate::function(move |p: &Path| p.to_str().unwrap().contains(&id.to_string())),
                predicate::eq(opt.file_size),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Box::pin(std::future::ready(Ok(()))));
        mock.expect_finalize()
            .with(predicate::eq(opt.id))
            .once()
            .in_sequence(&mut seq)
            .returning(|_| Box::pin(std::future::ready(Ok(()))));

        let (store, dir) = mock_fs_storage(mock);

        let (mut write, mut digest) = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size(), 0);

        write.write_all(content.as_bytes()).await.unwrap();
        digest.update(content.as_bytes());

        assert!(
            tokio::fs::try_exists(store.partial_file_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(write, &opt).await.unwrap();

        let digest = digest.finish();

        assert_eq!(digest.as_ref(), exp_digest.as_ref());

        let path = dir.path().join(id.to_string());
        let res = tokio::fs::read_to_string(&path).await.unwrap();

        assert_eq!(res, content);

        #[cfg(unix)]
        {
            let meta = tokio::fs::metadata(&path).await.unwrap();
            assert_eq!(meta.mode() & 0o7777, 0o600, "got {:#o}", meta.mode());
        }
    }

    #[tokio::test]
    async fn write_handle_new() {
        let (store, dir) = fs_storage();

        let id = Uuid::new_v4();

        let content = "Hello world";
        let exp_digest =
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, "Hello world".as_bytes());

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
            file_digest: FileDigest::Sha256,
            #[cfg(unix)]
            perm: FilePermissions {
                mode: Some(0o600),
                user_id: None,
                group_id: Some(100),
            },
            compression: None,
        };
        let (mut write, mut digest) = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size, 0);

        write.write_all(content.as_bytes()).await.unwrap();
        digest.update(content.as_bytes());

        assert!(
            tokio::fs::try_exists(store.partial_file_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(write, &opt).await.unwrap();

        let digest = digest.finish();

        assert_eq!(digest.as_ref(), exp_digest.as_ref());

        let path = dir.path().join(id.to_string());
        let res = tokio::fs::read_to_string(&path).await.unwrap();

        assert_eq!(res, content);

        #[cfg(unix)]
        {
            let meta = tokio::fs::metadata(&path).await.unwrap();
            assert_eq!(meta.mode() & 0o7777, 0o600, "got {:#o}", meta.mode());
        }
    }

    #[tokio::test]
    async fn write_handle_existing() {
        let (store, dir) = fs_storage();

        let id = Uuid::new_v4();

        let content = "Hello world";
        let content_1 = "Hello";
        let exp_digest =
            aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, "Hello world".as_bytes());

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
            file_digest: FileDigest::Sha256,
            #[cfg(unix)]
            perm: FilePermissions {
                mode: Some(0o600),
                user_id: None,
                group_id: None,
            },
            compression: None,
        };

        {
            let (mut write, _digest) = store.create_write_handle(&opt).await.unwrap();

            write.write_all(content_1.as_bytes()).await.unwrap();
            write.flush().await.unwrap();
        }

        // Write rest
        let (mut write, mut digest) = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size, content_1.len() as u64);

        let remaining = &content.as_bytes()[(write.current_size as usize)..];
        write.write_all(remaining).await.unwrap();
        digest.update(remaining);

        assert!(
            tokio::fs::try_exists(store.partial_file_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(write, &opt).await.unwrap();

        let digest = digest.finish();

        assert_eq!(digest.as_ref(), exp_digest.as_ref());

        let path = dir.path().join(id.to_string());
        let res = tokio::fs::read_to_string(&path).await.unwrap();

        assert_eq!(res, content);

        #[cfg(unix)]
        {
            let mode = tokio::fs::metadata(&path).await.unwrap().mode();
            assert_eq!(mode & 0o7777, 0o600, "got {mode:#o}");
        }
    }
}
