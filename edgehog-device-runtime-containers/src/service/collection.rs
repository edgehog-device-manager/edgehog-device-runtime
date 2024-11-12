// This file is part of Astarte.
//
// Copyright 2023-2024 SECO Mind Srl
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
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
};

use itertools::Itertools;
use petgraph::{
    stable_graph::{NodeIndex, StableDiGraph},
    Direction,
};
use tracing::{debug, error, instrument};

use crate::service::ServiceError;

use super::{node::Node, Id, Result};

/// Struct used to keep the collection of nodes and the relations between them.
///
/// It's a graph of the container resources as nodes and their dependencies as edges.
///
/// The Nodes struct is needed since on the [`Service`](super::Service) is needed to hold and access
/// mutably the state of the resources.
#[derive(Debug, Clone)]
pub(crate) struct NodeGraph {
    nodes: HashMap<Id, Node>,
    relations: Graph,
}

impl NodeGraph {
    pub(crate) fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            relations: Graph::new(),
        }
    }

    pub(crate) fn get_id(&self, idx: NodeIndex) -> Option<&Id> {
        self.relations.node_weight(idx)
    }

    /// Get the dependencies of a node
    pub(crate) fn deps(&self, node: &Node) -> impl Iterator<Item = &Id> {
        self.relations
            .neighbors_directed(node.idx, Direction::Outgoing)
            .filter_map(|dep| self.get_id(dep))
    }

    /// Get the ids of a node depending on the one provided
    pub(crate) fn dependent(&self, idx: NodeIndex) -> impl Iterator<Item = &Id> {
        self.relations
            .neighbors_directed(idx, Direction::Outgoing)
            .filter_map(|dep| self.get_id(dep))
    }

    /// Get a reference to a node
    pub(crate) fn node(&self, id: &Id) -> Option<&Node> {
        self.nodes.get(id).filter(|node| !node.state().is_missing())
    }
    /// Get a mutable reference to anode
    pub(crate) fn node_mut(&mut self, id: &Id) -> Option<&mut Node> {
        self.nodes
            .get_mut(id)
            .filter(|node| !node.state().is_missing())
    }

    pub(crate) fn add_node_sync<F>(&mut self, id: Id, deps: &[Id], f: F) -> Result<&mut Node>
    where
        F: FnOnce(Id, NodeIndex) -> Result<Node>,
    {
        let node_deps = self.create_node_deps(deps);

        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate(node.get().idx, node_deps.idxs())?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = *entry.key();

                self.relations.add_sync(id, node_deps.idxs(), |idx| {
                    let node = (f)(id, idx)?;

                    Ok(entry.insert(node))
                })
            }
        }
    }

    pub(crate) async fn add_node<F, O>(&mut self, id: Id, f: F) -> Result<&mut Node>
    where
        F: FnOnce(Id, NodeIndex) -> O,
        O: Future<Output = Result<Node>>,
    {
        self.add_node_with_deps(id, Vec::new(), |id, node_idx, _deps| (f)(id, node_idx))
            .await
    }

    pub(crate) async fn add_node_with_deps<F, O>(
        &mut self,
        id: Id,
        deps: Vec<Id>,
        f: F,
    ) -> Result<&mut Node>
    where
        F: FnOnce(Id, NodeIndex, Vec<Id>) -> O,
        O: Future<Output = Result<Node>>,
    {
        let node_deps = self.create_node_deps(&deps);

        let res = {
            match self.nodes.entry(id) {
                Entry::Occupied(entry) => self.relations.relate(entry.get().idx, node_deps.idxs()),
                Entry::Vacant(entry) => {
                    self.relations
                        .add(id, node_deps.idxs(), |idx| async move {
                            let node = (f)(id, idx, deps).await?;

                            entry.insert(node);

                            Ok(())
                        })
                        .await
                }
            }
        };

        match res {
            Ok(()) => {
                // FIXME: We could use the entry to return the mut node, but because of NLL we have
                //        a borrow error
                let node = self
                    .nodes
                    .get_mut(&id)
                    .expect("BUG: node was just inserted");
                Ok(node)
            }
            Err(err) => {
                self.cleanup_add_node(&id, &node_deps);

                Err(err)
            }
        }
    }

    fn cleanup_add_node(&mut self, id: &Id, deps: &NodeDeps) {
        for (deps_id, _) in &deps.missing {
            self.remove_node(deps_id);
        }

        self.remove_node(id);
    }

    /// Get or add as missing the node dependencies
    fn create_node_deps(&mut self, deps: &[Id]) -> NodeDeps {
        let present = deps
            .iter()
            .filter_map(|id| self.nodes.get(id).map(|node| (node.id, node.idx)))
            .collect();

        let missing = deps
            .iter()
            .filter_map(|id| self.add_missing(id))
            .collect_vec();

        NodeDeps { present, missing }
    }

    /// Add a missing node
    fn add_missing(&mut self, id: &Id) -> Option<(Id, NodeIndex)> {
        if self.nodes.contains_key(id) {
            return None;
        }

        let idx = self.relations.add_node(*id);
        self.nodes.insert(*id, Node::new(*id, idx));

        Some((*id, idx))
    }

    /// Removes a node and all the relations.
    fn remove_node(&mut self, id: &Id) {
        let Some(node) = self.nodes.remove(id) else {
            return;
        };

        self.relations.remove_node(node.idx);
    }

    pub(crate) fn relations(&self) -> &StableDiGraph<Id, ()> {
        &self.relations
    }

    pub(crate) fn nodes(&self) -> &HashMap<Id, Node> {
        &self.nodes
    }
}

impl Default for NodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Struct to support the addition of a node
#[derive(Debug, Clone)]
struct NodeDeps {
    /// Dependencies that where already present
    present: Vec<(Id, NodeIndex)>,
    /// Dependencies that where inserted because missing
    missing: Vec<(Id, NodeIndex)>,
}

impl NodeDeps {
    fn idxs(&self) -> impl Iterator<Item = &NodeIndex> + Clone {
        self.present.iter().map(|(_, idx)| idx)
    }
}

#[derive(Debug, Clone)]
struct Graph(StableDiGraph<Id, ()>);

impl Graph {
    fn new() -> Self {
        Self(StableDiGraph::new())
    }

    async fn add<'a, F, O, R, I>(&mut self, id: Id, deps: I, f: F) -> Result<R>
    where
        F: FnOnce(NodeIndex) -> O,
        O: Future<Output = Result<R>>,
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        let idx = self.add_node(id);

        let res = (f)(idx).await;

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn add_sync<'a, F, O, I>(&mut self, id: Id, deps: I, f: F) -> Result<O>
    where
        F: FnOnce(NodeIndex) -> Result<O>,
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        let idx = self.add_node(id);

        let res = (f)(idx);

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn relate_ok_or_rm<'a, R, I>(&mut self, res: Result<R>, idx: NodeIndex, deps: I) -> Result<R>
    where
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
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

    #[instrument(skip_all)]
    pub(crate) fn relate<'a, I>(&mut self, node: NodeIndex, deps: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        // We need to check each node or it will panic if not existing
        if !self.contains_node(node) {
            error!("node is missing");

            return Err(ServiceError::MissingRelation);
        }

        for dep in deps.clone() {
            if !self.contains_node(*dep) {
                error!("dependency {} is missing", dep.index());

                return Err(ServiceError::MissingRelation);
            }
        }

        for dep in deps {
            self.add_edge(node, *dep, ());
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
