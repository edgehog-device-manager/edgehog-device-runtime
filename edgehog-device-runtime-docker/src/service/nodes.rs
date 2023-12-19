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
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
};

use itertools::Itertools;
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
        deps: Vec<String>,
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> SResult<Node>,
    {
        let deps = self.get_node_deps(deps);

        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate_many(node.get().idx, deps.idxs())?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = entry.key().clone();

                self.relations.add_sync(id.clone(), deps.idxs(), |idx| {
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
        deps: Vec<String>,
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> O,
        O: Future<Output = SResult<Node>>,
    {
        let deps = self.get_node_deps(deps);

        let res = {
            match self.nodes.entry(id.clone()) {
                Entry::Occupied(entry) => self.relations.relate_many(entry.get().idx, deps.idxs()),
                Entry::Vacant(entry) => {
                    let id = id.clone();

                    self.relations
                        .add(id.clone(), deps.idxs(), |idx| async move {
                            let node = (f)(id, idx).await?;

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
                self.cleanup_add_node(&id, &deps);

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
    fn get_node_deps(&mut self, deps: Vec<String>) -> NodeDeps {
        let present = deps
            .iter()
            .filter_map(|id| {
                self.nodes
                    .get(id.as_str())
                    .map(|node| (node.id.clone(), node.idx))
            })
            .collect();

        let missing = deps
            .into_iter()
            .filter_map(|id| self.add_missing(id))
            .collect_vec();

        NodeDeps { present, missing }
    }

    /// Add a missing node
    fn add_missing(&mut self, id: String) -> Option<(Id, NodeIndex)> {
        if self.nodes.contains_key(id.as_str()) {
            return None;
        }

        let id = Id::new(id);

        let idx = self.relations.add_node(id.clone());
        self.nodes.insert(id.clone(), Node::new(id.clone(), idx));

        Some((id, idx))
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
}

impl Default for Nodes {
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

    async fn add<'a, F, O, R, I>(&mut self, id: Id, deps: I, f: F) -> SResult<R>
    where
        F: FnOnce(NodeIndex) -> O,
        O: Future<Output = SResult<R>>,
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        let idx = self.add_node(id);

        let res = (f)(idx).await;

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn add_sync<'a, F, O, I>(&mut self, id: Id, deps: I, f: F) -> SResult<O>
    where
        F: FnOnce(NodeIndex) -> SResult<O>,
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        let idx = self.add_node(id);

        let res = (f)(idx);

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn relate_ok_or_rm<'a, R, I>(&mut self, res: SResult<R>, idx: NodeIndex, deps: I) -> SResult<R>
    where
        I: IntoIterator<Item = &'a NodeIndex> + Clone,
    {
        let res = res.and_then(|r| {
            self.relate_many(idx, deps)?;

            Ok(r)
        });

        if res.is_err() {
            debug!("error adding node, removing relation");

            self.remove_node(idx);
        }

        res
    }

    #[instrument(skip(self))]
    pub(crate) fn relate(&mut self, node: NodeIndex, dep: NodeIndex) -> SResult<()> {
        // We need to check each node or it will panic if not existing
        if !self.contains_node(node) {
            error!("node is missing");

            return Err(ServiceError::MissingRelation);
        }

        if !self.contains_node(dep) {
            error!("dependency {} is missing", dep.index());

            return Err(ServiceError::MissingRelation);
        }

        self.add_edge(node, dep, ());

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) fn relate_many<'a, I>(&mut self, node: NodeIndex, deps: I) -> SResult<()>
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
