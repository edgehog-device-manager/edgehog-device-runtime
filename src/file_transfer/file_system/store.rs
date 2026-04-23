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

//! Handles file system operations for the file transfer.

use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};

use eyre::WrapErr;
use tracing::{instrument, trace};
use uuid::Uuid;

use crate::file_transfer::config::Percentage;
use crate::file_transfer::encoding::Paths;
use crate::file_transfer::request::FileDigest;

use super::{FileOptions, WriteHandle};

/// Stores files in the storage
#[derive(Debug)]
pub(crate) struct FileStorage<F> {
    dir: PathBuf,
    fs: F,
}

impl FileStorage<Fs> {
    pub(crate) fn new(dir: PathBuf, reserved: Percentage) -> Self {
        Self {
            fs: Fs { reserved },
            dir,
        }
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

    pub(crate) fn file_path(&self, id: &Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    #[instrument(skip(self), ret)]
    pub(crate) async fn file_exists(
        &self,
        id: &Uuid,
        alg: FileDigest,
        digest: &[u8],
    ) -> eyre::Result<bool> {
        let path = self.file_path(id);

        WriteHandle::try_exists(&path, alg, digest)
            .await
            .wrap_err_with(|| format!("couldn't access file: {}", path.display()))
    }

    #[instrument(skip(self))]
    pub(crate) async fn find_paths(&self, id: &Uuid) -> io::Result<Paths> {
        let path = self.file_path(id);

        Paths::read(path).await
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
    pub(crate) async fn create_write_handle(&self, opt: &FileOptions) -> io::Result<WriteHandle>
    where
        F: Space,
    {
        let file_path = self.file_path(&opt.id);

        self.fs
            .reserve_space(opt.id, &file_path, opt.file_size)
            .await?;

        trace!(path = %file_path.display(), "opening file for write");

        WriteHandle::open(file_path, opt).await
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    pub(crate) async fn finalize_write(
        &self,
        handle: &mut WriteHandle,
        opt: &FileOptions,
    ) -> io::Result<()>
    where
        F: Space,
    {
        handle.finalize(opt).await?;

        self.fs.finalize(opt.id).await?;

        Ok(())
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
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Marks the file as saved and refreshes the current quota
    fn finalize(&self, _id: Uuid) -> impl Future<Output = io::Result<()>> + Send;
}

#[derive(Debug)]
pub(crate) struct Fs {
    #[expect(unused)]
    reserved: Percentage,
}

impl Fs {}

impl Space for Fs {
    async fn reserve_space(&self, _id: Uuid, _path: &Path, _file_size: u64) -> io::Result<()> {
        // TODO: ensure 10% of the free space on disk

        Ok(())
    }

    async fn finalize(&self, _id: Uuid) -> io::Result<()> {
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
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};

    use crate::file_transfer::config::DEFAULT_MAX_FREE_PERCENTAGE;
    #[cfg(unix)]
    use crate::file_transfer::request::FilePermissions;

    use super::*;

    use pretty_assertions::assert_eq;

    fn partial_file_path(path: PathBuf) -> PathBuf {
        let mut path = path.into_os_string();

        path.push(".part");

        PathBuf::from(path)
    }

    fn fs_storage() -> (FileStorage<Fs>, TempDir) {
        let dir = TempDir::new("fs_storage").expect("couldn't create temp directory");

        (
            FileStorage::new(dir.path().to_path_buf(), DEFAULT_MAX_FREE_PERCENTAGE),
            dir,
        )
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

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
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

        let mut write = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size(), 0);

        write.write_all(content.as_bytes()).await.unwrap();

        assert!(
            tokio::fs::try_exists(partial_file_path(store.file_path(&id)))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

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

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
            #[cfg(unix)]
            perm: FilePermissions {
                mode: Some(0o600),
                user_id: None,
                group_id: Some(100),
            },
            compression: None,
        };
        let mut write = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size, 0);

        write.write_all(content.as_bytes()).await.unwrap();

        assert!(
            tokio::fs::try_exists(partial_file_path(store.file_path(&id)))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

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

        let opt = FileOptions {
            id,
            file_size: content.len().try_into().unwrap(),
            #[cfg(unix)]
            perm: FilePermissions {
                mode: Some(0o600),
                user_id: None,
                group_id: None,
            },
            compression: None,
        };

        {
            let mut write = store.create_write_handle(&opt).await.unwrap();

            write.write_all(&content.as_bytes()[..5]).await.unwrap();
            write.flush().await.unwrap();
        }

        // Write rest
        let mut write = store.create_write_handle(&opt).await.unwrap();

        assert_eq!(write.current_size, 5);

        write
            .seek(io::SeekFrom::Start(write.current_size))
            .await
            .unwrap();

        let remaining = &content.as_bytes()[5..];
        write.write_all(remaining).await.unwrap();

        assert!(
            tokio::fs::try_exists(partial_file_path(store.file_path(&id)))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

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
