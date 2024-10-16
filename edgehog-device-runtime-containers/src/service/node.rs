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

//! Node in the containers

use std::fmt::Debug;

use astarte_device_sdk::Client;
use petgraph::stable_graph::NodeIndex;
use tracing::instrument;

use crate::Docker;

use super::{resource::NodeType, state::State, Id, Result};

/// A node containing the [`State`], [`Id`] of the resource and index of the dependencies.
///
/// Its a node in the graph of container resources, with the dependencies of other nodes as edges.
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(super) id: Id,
    pub(super) idx: NodeIndex,
    state: State,
}

impl Node {
    pub(crate) fn new(id: Id, idx: NodeIndex) -> Self {
        Self {
            id,
            idx,
            state: State::Missing,
        }
    }

    pub(crate) fn with_state(id: Id, idx: NodeIndex, state: State) -> Self {
        Self { id, idx, state }
    }

    #[instrument(skip_all)]
    pub(super) async fn store<D, T>(&mut self, device: &D, inner: T) -> Result<()>
    where
        D: Debug + Client + Sync,
        T: Into<NodeType> + Debug,
    {
        self.state.store(&self.id, device, inner).await
    }

    #[instrument(skip_all)]
    pub(super) async fn up<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        self.state.up(&self.id, device, client).await
    }

    pub(crate) fn id(&self) -> &Id {
        &self.id
    }

    pub(crate) fn node_type(&self) -> Option<&NodeType> {
        match &self.state {
            State::Missing => None,
            State::Stored(nt) | State::Created(nt) | State::Up(nt) => Some(nt),
        }
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }
}
