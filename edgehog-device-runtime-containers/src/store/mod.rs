// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Persistent stores of the request issued by Astarte and resources created.

use std::borrow::Cow;
use std::io::{self, Cursor, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter};

use crate::service::{collection::Nodes, node::Node};

/// Error returned by the [`StateStore`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum StateStoreError {
    /// couldn't create the parent directory
    CreateDir(#[source] io::Error),
    /// couldn't open the state file
    Open(#[source] io::Error),
    /// couldn't load state
    Load(#[source] io::Error),
    /// couldn't append entry to state
    Append(#[source] io::Error),
    /// couldn't append store the state
    Store(#[source] io::Error),
    /// couldn't load state
    Serialize(#[source] serde_json::Error),
    /// couldn't load state
    Deserialize(#[source] serde_json::Error),
}

type Result<T> = std::result::Result<T, StateStoreError>;

/// Handle to persist the state.
///
/// The file is a new line delimited JSON.
#[derive(Debug)]
pub struct StateStore {
    file: BufWriter<File>,
}

impl StateStore {
    /// Opens the file to use as store.
    pub(crate) async fn open(file: impl AsRef<Path>) -> Result<Self> {
        let path = file.as_ref();

        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(StateStoreError::CreateDir)?;
        }

        let file = File::open(file).await.map_err(StateStoreError::Open)?;

        Ok(Self {
            file: BufWriter::new(file),
        })
    }

    /// Load the state from the persistence
    pub(crate) async fn load(&self) -> Result<()> {
        // The call to read is one at the beginning, so we don't need to keep the reader around
        let file = self
            .file
            .get_ref()
            .try_clone()
            .await
            .map_err(StateStoreError::Load)?;

        let mut reader = BufReader::new(file);
        let mut line = String::new();

        loop {
            let byte_read = reader
                .read_line(&mut line)
                .await
                .map_err(StateStoreError::Load)?;

            if byte_read == 0 {
                break;
            }

            // TODO: return or add it to the Nodes
            let value: Value = serde_json::from_str(&line).map_err(StateStoreError::Deserialize)?;
        }

        Ok(())
    }

    /// Appends the new struct to the state store
    pub(crate) async fn append<T>(&mut self, data: T) -> Result<()>
    where
        T: Serialize,
    {
        // At the end
        self.file
            .seek(SeekFrom::End(0))
            .await
            .map_err(StateStoreError::Append)?;

        let content = serde_json::to_string(&data).map_err(StateStoreError::Serialize)?;

        self.file
            .write_all(content.as_bytes())
            .await
            .map_err(StateStoreError::Append)?;
        self.file
            .write_u8(b'\n')
            .await
            .map_err(StateStoreError::Append)?;

        self.file.flush().await.map_err(StateStoreError::Append)?;

        Ok(())
    }

    /// Write all the state to the file
    pub(crate) async fn store(&mut self, state: &Nodes) -> Result<()> {
        // At the start and truncate
        self.file.rewind().await.map_err(StateStoreError::Store)?;
        self.file
            .get_mut()
            .set_len(0)
            .await
            .map_err(StateStoreError::Store)?;

        // Reuse the same allocation to store the serialized values
        let mut buf = Vec::new();
        let mut cursor = Cursor::new(&mut buf);

        for node in state.nodes().values() {
            let value = Value::from(node);

            serde_json::to_writer(&mut cursor, &value).map_err(StateStoreError::Serialize)?;

            self.file
                .write_all(cursor.get_ref())
                .await
                .map_err(StateStoreError::Store)?;
            self.file
                .write_u8(b'\n')
                .await
                .map_err(StateStoreError::Store)?;

            cursor.get_mut().clear();
            cursor.set_position(0);
        }

        self.file.flush().await.map_err(StateStoreError::Store)?;

        Ok(())
    }
}

/// State stored, includes the remote and local id of the resource
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Value<'a> {
    edgehog_id: Cow<'a, str>,
    local_id: Option<Cow<'a, str>>,
    started: bool,
    resource: Option<Resources>,
}

impl<'a> From<&'a Node> for Value<'a> {
    fn from(value: &'a Node) -> Self {
        let started = value.state().is_up();

        let (local_id, resource) = match value.node_type() {
            None => (None, None),
            Some(node_type) => unimplemented!(),
        };

        Self {
            edgehog_id: Cow::Borrowed(value.id().as_str()),
            started,
            local_id,
            resource,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Resources {}
