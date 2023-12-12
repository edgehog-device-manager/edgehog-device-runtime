// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Map of nodes and the relations between each other.

use std::{
    borrow::Borrow,
    collections::{hash_map::Entry, HashMap},
    future::Future,
    ops::{Deref, DerefMut},
};

use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use tracing::{debug, error, instrument};

use crate::service::ServiceError;

use super::{Id, Node, SResult};

/// Struct used to keep the collection of nodes and the relations between them.
///
/// The Nodes struct is needed since on the [`Service`] we need to have mixed mutable and immutable
/// reference to the various parts of the struct.
#[derive(Debug, Clone)]
pub(crate) struct Nodes {
    nodes: HashMap<Id, Node>,
    relations: Graph,
}

impl Nodes {
    pub(crate) fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            relations: Graph::new(),
        }
    }

    pub(crate) fn get_idx(&self, id: &str) -> Option<NodeIndex> {
        self.nodes.get(id).map(|node| node.idx)
    }

    pub(crate) fn get_id(&self, idx: NodeIndex) -> Option<&Id> {
        self.relations.node_weight(idx)
    }

    /// Get a reference to a node
    pub(crate) fn node(&mut self, id: &Id) -> Option<&Node> {
        self.nodes.get(id).filter(|node| !node.inner.is_missing())
    }
    /// Get a mutable reference to anode
    pub(crate) fn node_mut(&mut self, id: &Id) -> Option<&mut Node> {
        self.nodes
            .get_mut(id)
            .filter(|node| !node.inner.is_missing())
    }

    pub(crate) fn add_node_sync<F>(
        &mut self,
        id: Id,
        f: F,
        deps: &[NodeIndex],
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> SResult<Node>,
    {
        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate(node.get().idx, deps)?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = entry.key().clone();

                self.relations.add_sync(id.clone(), deps, |idx| {
                    let node = (f)(id, idx)?;

                    Ok(entry.insert(node))
                })
            }
        }
    }

    pub(crate) async fn add_node<F, O>(
        &mut self,
        id: Id,
        f: F,
        deps: &[NodeIndex],
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> O,
        O: Future<Output = Result<Node, ServiceError>>,
    {
        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate(node.get().idx, deps)?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = entry.key().clone();

                self.relations
                    .add(id.clone(), deps, |idx| async move {
                        let node = (f)(id, idx).await?;

                        Ok(entry.insert(node))
                    })
                    .await
            }
        }
    }

    pub(crate) fn relations(&self) -> &StableDiGraph<Id, ()> {
        &self.relations
    }
}

impl Default for Nodes {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct Graph(StableDiGraph<Id, ()>);

impl Graph {
    fn new() -> Self {
        Self(StableDiGraph::new())
    }

    async fn add<F, O, R>(&mut self, id: Id, deps: &[NodeIndex], f: F) -> SResult<R>
    where
        F: FnOnce(NodeIndex) -> O,
        O: Future<Output = SResult<R>>,
    {
        let idx = self.add_node(id);

        let res = (f)(idx).await;

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn add_sync<F, O>(&mut self, id: Id, deps: &[NodeIndex], f: F) -> SResult<O>
    where
        F: FnOnce(NodeIndex) -> SResult<O>,
    {
        let idx = self.add_node(id);

        let res = (f)(idx);

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn relate_ok_or_rm<R>(
        &mut self,
        res: SResult<R>,
        idx: NodeIndex,
        deps: &[NodeIndex],
    ) -> SResult<R> {
        let res = res.and_then(|r| {
            self.relate(idx, deps)?;

            Ok(r)
        });

        if res.is_err() {
            debug!("error adding node, removing relation");

            self.remove_node(idx);
        }

        res
    }

    #[instrument]
    pub(crate) fn relate(&mut self, node: NodeIndex, deps: &[NodeIndex]) -> SResult<()> {
        // We need to check each node or it will panic if not existing
        if !self.contains_node(node) {
            error!("node is missing");

            return Err(ServiceError::MissingRelation);
        }

        for dep in deps {
            let dep = *dep;
            if !self.contains_node(dep) {
                error!("dependency {} is missing", dep.index());

                return Err(ServiceError::MissingRelation);
            }

            self.add_edge(node, dep, ());
        }

        Ok(())
    }
}

impl Deref for Graph {
    type Target = StableDiGraph<Id, ()>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Graph {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Borrow<StableDiGraph<Id, ()>> for Graph {
    fn borrow(&self) -> &StableDiGraph<Id, ()> {
        &self.0
    }
}
