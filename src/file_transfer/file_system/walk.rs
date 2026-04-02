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

//! Walk a path returning each file and directory.

use std::path::PathBuf;
use std::sync::Arc;
use std::task::{Poll, ready};

use eyre::Context;
use futures::FutureExt;
use pin_project::pin_project;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::instrument;
use walkdir::WalkDir;

/// Iterates a directory recursively.
///
/// It will return all the files and directories, without following symbolic links and
///
/// We will use a depth first search for the directories.
#[derive(Debug)]
#[pin_project]
pub(crate) struct Walk {
    iter: Arc<Mutex<walkdir::IntoIter>>,
    handle: Option<JoinHandle<Option<eyre::Result<walkdir::DirEntry>>>>,
}

impl Walk {
    pub(crate) fn new(base_dir: PathBuf) -> Self {
        let iter = Arc::new(Mutex::new(
            WalkDir::new(base_dir).follow_links(false).into_iter(),
        ));

        Self { iter, handle: None }
    }
}

impl tokio_stream::Stream for Walk {
    type Item = eyre::Result<walkdir::DirEntry>;

    #[instrument(skip_all)]
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let handle = this.handle.get_or_insert_with(|| {
            let iter = Arc::clone(this.iter);

            tokio::task::spawn_blocking(move || {
                match iter.try_lock().wrap_err("couldn't borrow iterator") {
                    Ok(mut iter) => iter
                        .next()
                        .map(|res| res.wrap_err("couldn't read directory")),
                    Err(error) => Some(Err(error)),
                }
            })
        });

        let res = ready!(handle.poll_unpin(cx));

        this.handle.take();

        let res = res?;

        Poll::Ready(res)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use rstest::{fixture, rstest};
    use tempdir::TempDir;

    use super::*;

    #[fixture]
    pub(crate) async fn mk_dir_structure() -> TempDir {
        let dir = TempDir::new("mk_dir_structure").unwrap();

        let files = [
            "foo/root.txt",
            "foo/bar/a.txt",
            "foo/some/b.txt",
            "foo/some/other/c.txt",
        ];

        for file in files {
            let path = dir.path().join(file);

            tokio::fs::create_dir_all(path.parent().unwrap())
                .await
                .unwrap();

            tokio::fs::write(path, "").await.unwrap();
        }

        dir
    }

    #[fixture]
    pub(crate) async fn mk_empty_dir() -> TempDir {
        let dir = TempDir::new("mk_empty_dir").unwrap();

        let path = dir.path().join("empty");

        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();

        dir
    }

    #[rstest]
    #[timeout(Duration::from_secs(2))]
    #[tokio::test]
    async fn wal_directory(#[future] mk_dir_structure: TempDir) {
        let dir = mk_dir_structure.await;

        let walk = Walk::new(dir.path().join("foo"));

        let mut expected = [
            "foo/",
            "foo/bar/",
            "foo/some/",
            "foo/some/other/",
            "foo/root.txt",
            "foo/bar/a.txt",
            "foo/some/b.txt",
            "foo/some/other/c.txt",
        ]
        .map(|p| dir.path().join(p));

        expected.sort();

        let mut res: Vec<PathBuf> = walk.map(|entry| entry.unwrap().into_path()).collect().await;

        res.sort();

        assert_eq!(res, expected);
    }

    #[rstest]
    #[timeout(Duration::from_secs(2))]
    #[tokio::test]
    async fn empty_dir(#[future] mk_empty_dir: TempDir) {
        let dir = mk_empty_dir.await;

        let path = dir.path().join("empty");

        tokio::fs::create_dir_all(&path).await.unwrap();

        let walk = Walk::new(path);

        let mut expected = ["empty/"].map(|p| dir.path().join(p));

        expected.sort();

        let mut res: Vec<PathBuf> = walk.map(|entry| entry.unwrap().into_path()).collect().await;

        res.sort();

        assert_eq!(res, expected);
    }

    #[rstest]
    #[timeout(Duration::from_secs(2))]
    #[tokio::test]
    async fn path_to_file() {
        let dir = TempDir::new("path_to_file").unwrap();

        let path = dir.path().join("file.txt");
        tokio::fs::write(&path, "file").await.unwrap();

        let walk = Walk::new(path);

        let mut expected = ["file.txt"].map(|p| dir.path().join(p));

        expected.sort();

        let mut res: Vec<PathBuf> = walk.map(|entry| entry.unwrap().into_path()).collect().await;

        res.sort();

        assert_eq!(res, expected);
    }
}
