// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Structure to handle the SQLite store.
//!
//! ## Concurrency
//!
//! It handles concurrency by having a shared Mutex for the writer part and a per instance reader.
//! To have a new reader you need to open a new connection to the database.
//!
//! We pass a mutable reference to the connection to a [`FnOnce`]. If the closure panics the
//! connection will be lost and needs to be recreated.

use std::{
    error::Error,
    fmt::Debug,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use deadpool::managed::{BuildError, Pool, PoolError};
use diesel::{connection::SimpleConnection, Connection, ConnectionError, SqliteConnection};
use tokio::{sync::Mutex, task::JoinError};

type DynError = Box<dyn Error + Send + Sync>;
/// Result for the [`HandleError`] returned by the [`Handle`].
pub type Result<T> = std::result::Result<T, HandleError>;

/// Handler error
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum HandleError {
    /// couldn't open database with non UTF-8 path: {0}
    NonUtf8Path(PathBuf),
    /// couldn't join database task
    Join(#[from] JoinError),
    /// error returned while building the reader pool
    PoolBuilder(#[from] BuildError),
    /// error returned while creating the writer connection
    Writer(#[from] ManagerError),
    /// error returned while getting a reader connection
    Reader(#[from] PoolError<ManagerError>),
    /// couldn't execute the query
    Query(#[from] diesel::result::Error),
    /// couldn't run pending migrations
    Migrations(#[source] DynError),
    /// wrong number of rows updated, expected {exp} but modified {modified}
    UpdateRows {
        /// Number of rows modified.
        modified: usize,
        /// Expected number or rows.
        exp: usize,
    },
    /// error returned by the application
    #[error(transparent)]
    Application(DynError),
}

impl HandleError {
    /// Creates an [`HandleError::Application`] error.
    pub fn from_app(error: impl Into<DynError>) -> Self {
        Self::Application(error.into())
    }
}

impl HandleError {
    /// Check the result of the number of rows a query modified
    pub fn check_modified(modified: usize, exp: usize) -> Result<()> {
        if modified != exp {
            Err(HandleError::UpdateRows { exp, modified })
        } else {
            Ok(())
        }
    }
}

/// Read and write connection to the database
#[derive(Clone)]
pub struct Handle {
    /// Write handle to the database
    writer: Arc<Mutex<SqliteConnection>>,
    /// Per task/thread reader
    readers: Pool<Manager>,
}

impl Handle {
    /// Create a new instance by connecting to the file with default options
    pub async fn open(db_file: impl AsRef<Path>) -> Result<Self> {
        Self::with_options(db_file, SqliteOpts::default()).await
    }

    /// Create a new instance by connecting to the file
    pub async fn with_options(db_file: impl AsRef<Path>, options: SqliteOpts) -> Result<Self> {
        let db_path = db_file.as_ref();
        let db_str: String = db_path
            .to_str()
            .ok_or_else(|| HandleError::NonUtf8Path(db_path.to_path_buf()))
            .map(str::to_string)?;

        let manager = Manager {
            db_file: db_str,
            options,
        };

        let writer = manager.establish(false).await?;
        // We don't have migrations other than the containers for now
        #[cfg(feature = "containers")]
        let mut writer = writer;

        let writer = tokio::task::spawn_blocking(move || -> Result<SqliteConnection> {
            #[cfg(feature = "containers")]
            {
                use diesel_migrations::MigrationHarness;
                writer
                    .run_pending_migrations(crate::schema::CONTAINER_MIGRATIONS)
                    .map_err(HandleError::Migrations)?;
            }

            Ok(writer)
        })
        .await??;

        let readers = Pool::builder(manager)
            .max_size(options.max_pool_size.get())
            .build()?;

        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            readers,
        })
    }

    /// Passes the reader to a callback to execute a query.
    pub async fn for_read<F, O>(&self, f: F) -> Result<O>
    where
        F: FnOnce(&mut SqliteConnection) -> Result<O> + Send + 'static,
        O: Send + 'static,
    {
        let mut reader = self.readers.get().await?;

        // If this task panics (the error is returned) the connection would still be null
        let res = tokio::task::spawn_blocking(move || (f)(&mut reader)).await?;

        res
    }

    /// Passes the writer to a callback with a transaction already started.
    pub async fn for_write<F, O>(&self, f: F) -> Result<O>
    where
        F: FnOnce(&mut SqliteConnection) -> Result<O> + Send + 'static,
        O: Send + 'static,
    {
        let mut writer = Arc::clone(&self.writer).lock_owned().await;

        tokio::task::spawn_blocking(move || writer.transaction(|writer| (f)(writer))).await?
    }
}

impl Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("db_path", &self.readers.manager().db_file)
            .finish_non_exhaustive()
    }
}

/// Options for the SQLite connection
#[derive(Debug, Clone, Copy)]
pub struct SqliteOpts {
    max_pool_size: NonZeroUsize,
    busy_timout: Duration,
    cache_size: i16,
    max_page_count: u32,
    journal_size_limit: u64,
    wal_autocheckpoint: u32,
}

impl SqliteOpts {
    /// Setter for the max pool size
    pub fn set_max_pool_size(&mut self, max_pool_size: NonZeroUsize) {
        self.max_pool_size = max_pool_size;
    }

    /// Setter for the busy timeout
    pub fn set_busy_timout(&mut self, busy_timout: Duration) {
        self.busy_timout = busy_timout;
    }

    /// Setter for the max page count
    pub fn set_max_page_count(&mut self, max_page_count: u32) {
        self.max_page_count = max_page_count;
    }

    /// Setter for the journal size limit
    pub fn set_journal_size_limit(&mut self, journal_size_limit: u64) {
        self.journal_size_limit = journal_size_limit;
    }

    /// Setter for the WAL auto-checkpoint
    pub fn set_wal_autocheckpoint(&mut self, wal_autocheckpoint: u32) {
        self.wal_autocheckpoint = wal_autocheckpoint;
    }
}

impl Default for SqliteOpts {
    fn default() -> Self {
        const DEFAULT_POOL_SIZE: NonZeroUsize = match NonZeroUsize::new(4) {
            Some(size) => size,
            None => unreachable!(),
        };
        // 2 gib (assumes 4096 page size)
        const DEFAULT_MAX_PAGE_COUNT: u32 = 2 * (1024 * 1024 * 1024) / 4096;

        Self {
            max_pool_size: std::thread::available_parallelism().unwrap_or(DEFAULT_POOL_SIZE),
            busy_timout: Duration::from_secs(5),
            // 2 kib
            cache_size: -2 * 1024,
            // 2 gib (assumes 4096 page size)
            max_page_count: DEFAULT_MAX_PAGE_COUNT,
            // 64 mib
            journal_size_limit: 64 * 1024 * 1024,
            // 1000 pages
            wal_autocheckpoint: 1000,
        }
    }
}

struct Manager {
    db_file: String,
    options: SqliteOpts,
}

impl Manager {
    async fn establish(&self, reader: bool) -> std::result::Result<SqliteConnection, ManagerError> {
        let options = self.options;
        let db_file = self.db_file.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn =
                SqliteConnection::establish(&db_file).map_err(|err| ManagerError::Connection {
                    db_file: db_file.to_string(),
                    backtrace: err,
                })?;

            conn.batch_execute("PRAGMA journal_mode = wal;")?;
            conn.batch_execute("PRAGMA foreign_keys = true;")?;
            conn.batch_execute("PRAGMA synchronous = NORMAL;")?;
            conn.batch_execute("PRAGMA auto_vacuum = INCREMENTAL;")?;
            conn.batch_execute("PRAGMA temp_store = MEMORY;")?;
            // NOTE: Safe to format since we handle the options, do not pass strings.
            conn.batch_execute(&format!(
                "PRAGMA busy_timeout = {};",
                options.busy_timout.as_millis()
            ))?;
            conn.batch_execute(&format!("PRAGMA cache_size = {};", options.cache_size))?;
            conn.batch_execute(&format!(
                "PRAGMA max_page_count = {};",
                options.max_page_count
            ))?;
            conn.batch_execute(&format!(
                "PRAGMA journal_size_limit = {};",
                options.journal_size_limit
            ))?;
            conn.batch_execute(&format!(
                "PRAGMA wal_autocheckpoint = {};",
                options.wal_autocheckpoint
            ))?;

            if reader {
                conn.batch_execute("PRAGMA query_only = ON;")?;
            }

            Ok(conn)
        })
        .await?
    }
}

impl deadpool::managed::Manager for Manager {
    type Type = diesel::sqlite::SqliteConnection;

    type Error = ManagerError;

    async fn create(&self) -> std::result::Result<Self::Type, Self::Error> {
        self.establish(true).await
    }

    async fn recycle(
        &self,
        _obj: &mut Self::Type,
        _metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
        Ok(())
    }
}

/// Error returned while creating a connection
#[derive(Debug, thiserror::Error, displaydoc::Display)]
#[non_exhaustive]
pub enum ManagerError {
    /// couldn't connect to the database {db_file}
    Connection {
        /// Connection to the database file
        db_file: String,
        /// Underling connection error
        #[source]
        backtrace: ConnectionError,
    },
    /// couldn't join database task
    Join(#[from] JoinError),
    /// couldn't execute the query
    Query(#[from] diesel::result::Error),
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn should_open_db() {
        let tmp = TempDir::with_prefix("should_open").unwrap();

        Handle::open(&tmp.path().join("database.db")).await.unwrap();
    }
}
