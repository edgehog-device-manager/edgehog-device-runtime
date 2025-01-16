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
    path::{Path, PathBuf},
    sync::Arc,
};

use diesel::{sql_query, Connection, ConnectionError, RunQueryDsl, SqliteConnection};
use diesel_migrations::MigrationHarness;
use sync_wrapper::SyncWrapper;
use tokio::{sync::Mutex, task::JoinError};
use tracing::debug;

type DynError = Box<dyn Error + Send + Sync>;
/// Result for the [`HandleError`] returned by the [`Handle`].
pub type Result<T> = std::result::Result<T, HandleError>;

/// PRAGMA for the connection
const INIT_READER: &str = include_str!("../assets/init-reader.sql");
const INIT_WRITER: &str = include_str!("../assets/init-writer.sql");

/// Handler error
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum HandleError {
    /// couldn't open database with non UTF-8 path: {0}
    NonUtf8Path(PathBuf),
    /// couldn't join database task
    Join(#[from] JoinError),
    /// couldn't connect to the database {db_file}
    Connection {
        /// Connection to the database file
        db_file: String,
        /// Underling connection error
        #[source]
        backtrace: ConnectionError,
    },
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
pub struct Handle {
    db_file: String,
    /// Write handle to the database
    pub writer: Arc<Mutex<SqliteConnection>>,
    /// Per task/thread reader
    // NOTE: this is needed because the connection isn't Sync, and we need to pass the Connection
    //       to another thread (for tokio). The option signal if the connection was invalidated by
    //       the inner task panicking. In that case we re-create the reader connection.
    pub reader: SyncWrapper<Option<Box<SqliteConnection>>>,
}

impl Handle {
    /// Create a new instance by connecting to the file
    pub async fn open(db_file: impl AsRef<Path>) -> Result<Self> {
        let db_path = db_file.as_ref();
        let db_str = db_path
            .to_str()
            .ok_or_else(|| HandleError::NonUtf8Path(db_path.to_path_buf()))?;

        let mut writer = Self::establish_writer(db_str).await?;

        let writer = tokio::task::spawn_blocking(move || -> Result<SqliteConnection> {
            #[cfg(feature = "containers")]
            writer
                .run_pending_migrations(crate::schema::CONTAINER_MIGRATIONS)
                .map_err(HandleError::Migrations)?;

            Ok(writer)
        })
        .await??;

        let writer = Arc::new(Mutex::new(writer));
        let reader = Self::establish_reader(db_str).await?;

        Ok(Self {
            db_file: db_str.to_string(),
            writer,
            reader: SyncWrapper::new(Some(Box::new(reader))),
        })
    }

    /// Sets options for the connection
    async fn establish_writer(db_file: &str) -> Result<SqliteConnection> {
        establish(db_file, INIT_WRITER).await
    }

    /// Sets options for the connection
    async fn establish_reader(db_file: &str) -> Result<SqliteConnection> {
        establish(db_file, INIT_READER).await
    }

    /// Create a new handle for the store
    pub async fn clone_handle(&self) -> Result<Self> {
        let reader = Self::establish_reader(&self.db_file).await?;

        Ok(Self {
            db_file: self.db_file.clone(),
            writer: Arc::clone(&self.writer),
            reader: SyncWrapper::new(Some(Box::new(reader))),
        })
    }

    /// Create a new handle for the store, it will not initialize the reader connection.
    pub fn clone_lazy(&self) -> Self {
        Self {
            db_file: self.db_file.clone(),
            writer: Arc::clone(&self.writer),
            reader: SyncWrapper::new(None),
        }
    }

    /// Passes the reader to a callback to execute a query.
    pub async fn for_read<F, O>(&mut self, f: F) -> Result<O>
    where
        F: FnOnce(&mut SqliteConnection) -> Result<O> + Send + 'static,
        O: Send + 'static,
    {
        // Take the reader to move it to blocking task
        let mut reader = match self.reader.get_mut().take() {
            Some(reader) => reader,
            None => {
                debug!(
                    "connection missing, establishing a new one to {}",
                    self.db_file
                );

                Self::establish_reader(&self.db_file).await.map(Box::new)?
            }
        };

        // If this task panics (the error is returned) the connection would still be null
        let (reader, res) = tokio::task::spawn_blocking(move || {
            let res = (f)(&mut reader);

            (reader, res)
        })
        .await?;

        *self.reader.get_mut() = Some(reader);

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

async fn establish(
    db_file: &str,
    pragma: &'static str,
) -> std::result::Result<SqliteConnection, HandleError> {
    let mut conn = SqliteConnection::establish(db_file).map_err(|err| HandleError::Connection {
        db_file: db_file.to_string(),
        backtrace: err,
    })?;

    tokio::task::spawn_blocking(move || {
        sql_query(pragma)
            .execute(&mut conn)
            .map_err(HandleError::Query)?;

        Ok(conn)
    })
    .await?
}

impl Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("db_file", &self.db_file)
            .finish_non_exhaustive()
    }
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
