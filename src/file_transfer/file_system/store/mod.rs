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

use std::ffi::OsStr;
use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};

use cfg_if::cfg_if;
use edgehog_store::models::job::job_type::JobType;
use eyre::WrapErr;
use futures::{Stream, TryStreamExt};
use tokio::fs::{DirEntry, read_dir};
use tokio_stream::wrappers::ReadDirStream;
use tracing::{error, instrument, trace, warn};
use uuid::Uuid;

use crate::file_transfer::config::Percentage;
use crate::file_transfer::encoding::Paths;
use crate::file_transfer::interface::file::StoredFile;
use crate::file_transfer::request::{FileDigest, TransferJobTag};
use crate::jobs::Queue;

use super::{FileOptions, WriteHandle};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// Stores files in the storage
#[derive(Debug)]
pub(crate) struct FileStorage<F> {
    dir: PathBuf,
    fs: F,
}

impl FileStorage<()> {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { fs: (), dir }
    }
}

impl FileStorage<Fs> {
    pub(crate) fn with_reserved(dir: PathBuf, reserved: Percentage) -> Self {
        Self {
            fs: Fs { reserved },
            dir,
        }
    }
}

impl<F> FileStorage<F> {
    #[instrument(skip_all)]
    pub(crate) async fn init(&self, queue: &Queue) -> eyre::Result {
        trace!(path = %self.dir.display(), "initialazing store dir");

        tokio::fs::create_dir_all(&self.dir)
            .await
            .wrap_err("couldn't create file storage directory")?;

        self.cleanup(queue).await?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn cleanup(&self, queue: &Queue) -> eyre::Result {
        let mut dir = tokio::fs::read_dir(&self.dir)
            .await
            .wrap_err("couldn't read file storage directory")?;

        while let Ok(Some(entry)) = dir
            .next_entry()
            .await
            .inspect_err(|error| error!(%error,"couldn't get next dir entry"))
        {
            let path = entry.path();

            if Self::cleanup_symlinks(&entry, &path).await {
                continue;
            }

            if Self::cleanup_partial(&path, queue).await {
                continue;
            }

            self.adapt_entry(&entry).await?;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn adapt_entry(&self, entry: &DirEntry) -> eyre::Result {
        let name = entry.file_name();

        if !entry.file_type().await?.is_file() {
            // ignore all entries that are not files
            trace!(name = %name.display(), "ignoring entry: not a file");
            return Ok(());
        }

        let Some(name) = name.as_os_str().to_str() else {
            // ignore entry that don't have a valid name
            trace!(name = %name.display(), "ignoring entry: not a valid name");
            return Ok(());
        };

        let uuid = match Uuid::try_parse(name) {
            Ok(i) => i,
            Err(error) => {
                // ignore entry that don't have a valid uuid as name
                trace!(%error, name, "ignoring entry: not a valid uuid");
                return Ok(());
            }
        };

        let file = entry.path();
        let temp_dir = self.dir.join(Path::new(&format!("{uuid}_temp")));
        let dest_file = temp_dir.join(WriteHandle::MIGRATION_FILE_NAME);

        trace!(
            %uuid,
            file = %file.display(),
            temp = %dest_file.display(),
            "migrating file entry using temp dir",
        );

        // create temp directory
        tokio::fs::create_dir_all(&temp_dir).await?;
        // move file inside temp directory
        tokio::fs::rename(&file, dest_file).await?;
        // move temp directory to previous file location
        tokio::fs::rename(&temp_dir, &file).await?;

        Ok(())
    }

    /// Cleanup symlinks
    /// if the file type can't be checked the file won't be cleaned up
    #[instrument(skip_all)]
    async fn cleanup_symlinks(entry: &DirEntry, path: &Path) -> bool {
        let file_type = match entry.file_type().await {
            Ok(t) => t,
            Err(error) => {
                error!(%error, path = %path.display(), "can't check file type");

                return false;
            }
        };

        if !file_type.is_symlink() {
            return false;
        }

        if let Err(error) = tokio::fs::remove_file(path).await {
            error!(%error, path = %path.display(), "couldn't remove symlink");
        }

        true
    }

    /// Cleanup partial files whose job does not exist
    #[instrument(skip_all)]
    async fn cleanup_partial(path: &Path, queue: &Queue) -> bool {
        let Some(id) = Self::get_partial_id(path) else {
            trace!(entry = %path.display(), "not a partial file");
            return false;
        };

        let res = queue
            .exists(&id, JobType::FileTransfer, TransferJobTag::Download.into())
            .await;

        let job_exists = match res {
            Ok(exists) => exists,
            Err(error) => {
                error!(%error, "couldn't check if job exists");

                return true;
            }
        };

        if job_exists {
            return true;
        }

        if let Err(error) = tokio::fs::remove_file(path).await {
            error!(%error, path = %path.display(), "couldn't remove partial file")
        }

        return true;
    }

    #[instrument]
    fn get_partial_id(entry: &Path) -> Option<Uuid> {
        let file_steam = entry.file_stem()?;
        let file_ext = entry.extension()?;

        if file_ext != OsStr::new(WriteHandle::PARTIAL) {
            return None;
        }

        let file_steam = file_steam.to_str()?;

        Uuid::parse_str(file_steam)
            .inspect_err(|error| error!(%error, "couldn't parse partial id"))
            .ok()
    }

    pub(crate) fn dir_path(&self, id: &Uuid) -> PathBuf {
        self.dir.join(id.to_string())
    }

    pub(crate) fn partial_path(&self, id: &Uuid) -> PathBuf {
        let partial_file_name = format!("{}{}", id, WriteHandle::PARTIAL_EXT);
        self.dir.join(partial_file_name)
    }

    pub(crate) fn file_path(&self, id: &Uuid, name: &Path) -> PathBuf {
        self.dir_path(id).join(name)
    }

    #[instrument(skip(self), ret)]
    pub(crate) async fn file_exists(
        &self,
        id: &Uuid,
        file_name: &Path,
        alg: FileDigest,
        digest: &[u8],
    ) -> eyre::Result<bool> {
        let path = self.file_path(id, file_name);

        WriteHandle::try_exists(&path, alg, digest)
            .await
            .wrap_err_with(|| format!("couldn't access file: {}", path.display()))
    }

    #[instrument(skip(self))]
    pub(crate) async fn find_paths(&self, id: &Uuid) -> io::Result<Paths> {
        let path = self.dir_path(id);

        Paths::read(path).await
    }

    #[instrument(skip(self))]
    pub(crate) async fn inner_file_path(&self, id: &Uuid) -> io::Result<PathBuf> {
        let dir = self.dir_path(id);

        trace!(path = %dir.display(), "looking for file in directory");

        let mut read_dir = read_dir(&dir).await?;

        let inner = read_dir.next_entry().await?;

        let Some(inner) = inner else {
            error!(path = %dir.display(), "dir empty");

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a single file in store dir, found none",
            ));
        };

        if read_dir.next_entry().await?.is_some() {
            error!(path = %dir.display(), "dir contains more than one element");

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected a single file in store dir, found > 1",
            ));
        }

        return Ok(inner.path());
    }

    #[instrument(skip(self))]
    pub(crate) async fn open_read(&self, id: &Uuid) -> eyre::Result<tokio::fs::File> {
        let file_path = self.inner_file_path(id).await?;

        trace!(path = %file_path.display(), "opening file for read");

        // TODO: flock the file?
        let file = tokio::fs::File::options()
            .read(true)
            .open(file_path)
            .await?;

        Ok(file)
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    pub(crate) async fn create_write_handle(
        &mut self,
        name: &Path,
        opt: &FileOptions,
    ) -> io::Result<WriteHandle>
    where
        F: Space,
    {
        let file = self.file_path(&opt.id, name);
        let partial = self.partial_path(&opt.id);
        let handle = WriteHandle::open(file, partial, opt).await?;

        self.fs
            .reserve_space(opt.id, &handle.partial, opt.file_size)
            .await?;

        Ok(handle)
    }

    #[instrument(skip_all, fields(id = %opt.id))]
    pub(crate) async fn finalize_write(
        &mut self,
        handle: &mut WriteHandle,
        opt: &FileOptions,
    ) -> io::Result<StoredFile>
    where
        F: Space,
    {
        handle.finalize(opt).await?;

        self.fs.finalize(opt.id, &handle.path).await?;

        let id = opt.id;
        let size = opt.file_size;
        let path = handle.path.clone();

        Ok(StoredFile::create(id.to_string(), path, size))
    }

    #[instrument(skip_all)]
    pub(crate) async fn files(&self) -> io::Result<impl Stream<Item = io::Result<StoredFile>>> {
        let read_dir = tokio::fs::read_dir(&self.dir).await?;

        let stream = ReadDirStream::new(read_dir).try_filter_map(async |e| {
            let name = e.file_name();
            let name = name.to_string_lossy();

            if name.ends_with(WriteHandle::PARTIAL_EXT) {
                return Ok(None);
            }

            if !e.file_type().await?.is_dir() {
                error!(%name, "unexpected file in store directory");
                return Ok(None);
            }

            let path = e.path();
            // TODO compute a correct file size for the entry in the store
            // this requires walking the directory structure
            // let size = e.metadata().await?.len();

            Ok(Some(StoredFile::create(name.into(), path, 0)))
        });

        Ok(stream)
    }
}

#[cfg_attr(test, mockall::automock)]
pub(crate) trait Space {
    /// Reserves the space for the file on the device.
    ///
    /// It will make sure that at least the 10% of free space is available on the device the files
    /// are stored on.
    fn reserve_space(
        &mut self,
        id: Uuid,
        path: &Path,
        file_size: u64,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Marks the file as saved and refreshes the current quota
    fn finalize(&mut self, id: Uuid, path: &Path) -> impl Future<Output = io::Result<()>> + Send;
}

#[derive(Debug)]
pub(crate) struct Fs {
    reserved: Percentage,
}

impl Fs {
    fn has_avail(&self, mut file_size: u64, stat: &FsStat) -> bool {
        // Use next multiple of allocation_granularity
        let rem = file_size % stat.fragment_size;
        file_size = file_size.saturating_add(stat.fragment_size.saturating_sub(rem));

        let reserved = self.reserved.calculate(stat.fs_total);

        reserved < stat.user_avail.saturating_sub(file_size)
    }

    fn has_total_free(&self, stat: &FsStat) -> bool {
        let reserved = self.reserved.calculate(stat.fs_total);

        reserved < stat.user_avail
    }
}

// TODO: should use tokio here
impl Space for Fs {
    // TODO: we could pre-allocate the file size?
    #[instrument(skip(self, path))]
    async fn reserve_space(&mut self, id: Uuid, path: &Path, file_size: u64) -> io::Result<()> {
        let stat = FsStat::read(path)?;

        if !self.has_avail(file_size, &stat) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "file size exceeds the required reserved free space",
            ));
        }

        Ok(())
    }

    // TODO: we should cleanup the file
    #[instrument(skip(self, path))]
    async fn finalize(&mut self, id: Uuid, path: &Path) -> io::Result<()> {
        let stat = FsStat::read(path)?;

        if !self.has_total_free(&stat) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "file size exceeds the required reserved free space",
            ));
        }

        Ok(())
    }
}

/// Information of a mounted filesystem.
///
/// We use bytes to overcome cross system compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FsStat {
    /// Fragment size
    ///
    /// Size of a single contiguous fragment of data that can be stored.
    fragment_size: u64,
    /// Filesystem user space.
    ///
    /// Space on fs to store data for an unprivileged user, calculated as multiple of fragment size.
    user_avail: u64,
    /// Total filesystem space.
    ///
    /// Total filesystem space calculated as a multiple of fragment size.
    fs_total: u64,
}

impl FsStat {
    fn read(path: &Path) -> io::Result<Self> {
        cfg_if! {
            if #[cfg(windows)] {
                self::windows::get_disk_free_space(path)
            } else {
                self::unix::read_statvfs(path)
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    use mockall::{Sequence, predicate};
    use tempdir::TempDir;
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};

    #[cfg(unix)]
    use crate::file_transfer::request::FilePermissions;

    use super::*;

    use pretty_assertions::assert_eq;

    // NOTE: GitHub windows runner have a little less than 20% of free space available. So to not
    // have the CI failing, we stay well under that limit with a 10% reserved space
    pub(crate) const TEST_RESERVED_PERCENTAGE: Percentage = Percentage::new(10).unwrap();

    fn fs_storage() -> (FileStorage<Fs>, TempDir) {
        let dir = TempDir::new("fs_storage").expect("couldn't create temp directory");

        (
            FileStorage::with_reserved(dir.path().to_path_buf(), TEST_RESERVED_PERCENTAGE),
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
        let file_name = Path::new("testfile.txt");

        let id = Uuid::new_v4();
        let dir_path = dir.path().join(id.to_string());
        let file_path = dir.path().join(id.to_string()).join(file_name);

        assert_eq!(store.file_path(&id, file_name), file_path);

        tokio::fs::create_dir_all(dir_path).await.unwrap();
        tokio::fs::write(&file_path, "test").await.unwrap();

        let inner = store.inner_file_path(&id).await.unwrap();

        assert_eq!(inner, file_path);
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
            .with(
                predicate::eq(opt.id),
                predicate::function(move |p: &Path| p.to_str().unwrap().contains(&id.to_string())),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _| Box::pin(std::future::ready(Ok(()))));

        let (mut store, _dir) = mock_fs_storage(mock);

        let mut write = store
            .create_write_handle(Path::new("testfile.txt"), &opt)
            .await
            .unwrap();

        assert_eq!(write.current_size(), 0);

        write.write_all(content.as_bytes()).await.unwrap();

        assert!(
            tokio::fs::try_exists(store.partial_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

        let path = store.inner_file_path(&id).await.unwrap();
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
        let (mut store, _dir) = fs_storage();

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
        let mut write = store
            .create_write_handle(Path::new("testfile.txt"), &opt)
            .await
            .unwrap();

        assert_eq!(write.current_size, 0);

        write.write_all(content.as_bytes()).await.unwrap();

        assert!(
            tokio::fs::try_exists(store.partial_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

        let path = store.inner_file_path(&id).await.unwrap();
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
        let (mut store, _dir) = fs_storage();

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
            let mut write = store
                .create_write_handle(Path::new("testfile.txt"), &opt)
                .await
                .unwrap();

            write.write_all(&content.as_bytes()[..5]).await.unwrap();
            write.flush().await.unwrap();
        }

        // Write rest
        let mut write = store
            .create_write_handle(Path::new("testfile.txt"), &opt)
            .await
            .unwrap();

        assert_eq!(write.current_size, 5);

        write
            .seek(io::SeekFrom::Start(write.current_size))
            .await
            .unwrap();

        let remaining = &content.as_bytes()[5..];
        write.write_all(remaining).await.unwrap();

        assert!(
            tokio::fs::try_exists(store.partial_path(&id))
                .await
                .unwrap()
        );

        store.finalize_write(&mut write, &opt).await.unwrap();

        let path = store.inner_file_path(&id).await.unwrap();
        let res = tokio::fs::read_to_string(&path).await.unwrap();

        assert_eq!(res, content);

        #[cfg(unix)]
        {
            let mode = tokio::fs::metadata(&path).await.unwrap().mode();
            assert_eq!(mode & 0o7777, 0o600, "got {mode:#o}");
        }
    }
}
