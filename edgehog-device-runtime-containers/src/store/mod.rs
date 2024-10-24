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

use std::hash::Hash;
use std::io::{self, Cursor, SeekFrom};
use std::path::Path;

use container::ContainerState;
use image::ImageState;
use itertools::Itertools;
use network::NetworkState;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, BufWriter};
use tracing::{debug, instrument};
use volume::VolumeState;

use crate::container::Container;
use crate::image::Image;
use crate::network::Network;
use crate::service::{collection::NodeGraph, node::Node, resource::NodeType, Id};
use crate::volume::Volume;

pub(crate) mod container;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

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
    pub async fn open(file: impl AsRef<Path>) -> Result<Self> {
        let path = file.as_ref();

        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir)
                .await
                .map_err(StateStoreError::CreateDir)?;
        }

        let file = File::options()
            .append(true)
            .read(true)
            .write(true)
            .create(true)
            .open(file)
            .await
            .map_err(StateStoreError::Open)?;

        Ok(Self {
            file: BufWriter::new(file),
        })
    }

    /// Load the state from the persistence
    pub(crate) async fn load(&self) -> Result<Vec<Value>> {
        // The call to read is one at the beginning, so we don't need to keep the reader around
        let file = self
            .file
            .get_ref()
            .try_clone()
            .await
            .map_err(StateStoreError::Load)?;

        let mut reader = BufReader::new(file);
        let mut line = String::new();

        let mut values = Vec::new();

        loop {
            let byte_read = reader
                .read_line(&mut line)
                .await
                .map_err(StateStoreError::Load)?;

            if byte_read == 0 {
                break;
            }

            let value: Value = serde_json::from_str(&line).map_err(StateStoreError::Deserialize)?;

            values.push(value);
            line.clear();
        }

        Ok(values)
    }

    /// Appends the new struct to the state store
    #[instrument(skip(resource, deps))]
    pub(crate) async fn append(
        &mut self,
        id: Id,
        resource: Resource<'_>,
        deps: Vec<Id>,
    ) -> Result<()> {
        debug!("appending resource");

        // At the end
        self.file
            .seek(SeekFrom::End(0))
            .await
            .map_err(StateStoreError::Append)?;

        let resource = Value::with_resource(id, deps, resource);

        let content = serde_json::to_string(&resource).map_err(StateStoreError::Serialize)?;

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
    #[instrument(skip_all, fields(nodes = %state.nodes().len()))]
    pub(crate) async fn store(&mut self, state: &NodeGraph) -> Result<()> {
        debug!("storing state");

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
            let deps = state.deps(node).cloned().collect_vec();

            let value = Value::from_node(node, deps);

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
pub(crate) struct Value<'a> {
    // Id provided by Edgehog
    pub(crate) id: Id,
    pub(crate) deps: Vec<Id>,
    pub(crate) resource: Option<Resource<'a>>,
}

impl<'a> Value<'a> {
    fn new(id: Id, deps: Vec<Id>, resource: Option<Resource<'a>>) -> Self {
        Self { id, deps, resource }
    }

    fn with_resource(id: Id, deps: Vec<Id>, resource: Resource<'a>) -> Self {
        Self::new(id, deps, Some(resource))
    }

    fn from_node(value: &'a Node, deps: Vec<Id>) -> Self {
        let resource = value.node_type().map(Resource::from);

        Self::new(value.id().clone(), deps, resource)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum Resource<'a> {
    Image(ImageState<'a>),
    Volume(VolumeState<'a>),
    Network(NetworkState<'a>),
    Container(ContainerState<'a>),
    Deployment,
}

impl<'a, S> From<&'a Image<S>> for Resource<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Image<S>) -> Self {
        Resource::Image(value.into())
    }
}

impl<'a, S> From<&'a Volume<S>> for Resource<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Volume<S>) -> Self {
        Resource::Volume(value.into())
    }
}

impl<'a, S> From<&'a Network<S>> for Resource<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Network<S>) -> Self {
        Resource::Network(value.into())
    }
}

impl<'a, S> From<&'a Container<S>> for Resource<'a>
where
    S: AsRef<str> + Eq + Hash,
{
    fn from(value: &'a Container<S>) -> Self {
        Resource::Container(value.into())
    }
}

impl<'a> From<&'a NodeType> for Resource<'a> {
    fn from(value: &'a NodeType) -> Self {
        match value {
            NodeType::Image(image) => Resource::from(image),
            NodeType::Volume(volume) => Resource::from(volume),
            NodeType::Network(network) => Resource::from(network),
            NodeType::Container(container) => Resource::from(container),
            NodeType::Deployment => Resource::Deployment,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    async fn open_tmp() -> (StateStore, TempDir) {
        let tmpdir = TempDir::new().unwrap();

        let file = tmpdir.path().join("store.json");
        let store = StateStore::open(&file).await.unwrap();

        (store, tmpdir)
    }

    #[tokio::test]
    async fn should_open() {
        let (_store, tmpdir) = open_tmp().await;

        let exists = tokio::fs::try_exists(&tmpdir.path().join("store.json"))
            .await
            .unwrap();

        assert!(exists);
    }

    #[tokio::test]
    async fn should_load() {
        let (store, _) = open_tmp().await;

        store.load().await.unwrap();
    }

    #[tokio::test]
    async fn should_store() {
        let (mut store, _) = open_tmp().await;

        store.store(&NodeGraph::new()).await.unwrap();
        store.load().await.unwrap();
    }
}
