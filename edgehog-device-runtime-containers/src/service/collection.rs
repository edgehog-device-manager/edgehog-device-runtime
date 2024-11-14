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

use std::{collections::HashMap, fmt::Debug};

use petgraph::{
    stable_graph::{NodeIndex, StableDiGraph},
    visit::Walker,
    Direction,
};
use tracing::debug;

use super::{node::Node, resource::NodeResource, Id, ServiceError};

type Graph = StableDiGraph<Id, ()>;

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

    /// Get the ids of nodes depending on the one provided
    pub(crate) fn dependent(&self, idx: NodeIndex) -> impl Iterator<Item = &Id> {
        self.relations
            .neighbors_directed(idx, Direction::Outgoing)
            .filter_map(|dep| self.get_id(dep))
    }

    /// Get a reference to a node
    pub(crate) fn node(&self, id: &Id) -> Option<&Node> {
        self.nodes.get(id).filter(|node| node.resource.is_some())
    }

    /// Get a mutable reference to anode
    pub(crate) fn node_mut(&mut self, id: &Id) -> Option<&mut Node> {
        self.nodes
            .get_mut(id)
            .filter(|node| node.resource.is_some())
    }

    fn insert(&mut self, id: Id, resource: Option<NodeResource>, deps: &[Id]) -> &mut Node {
        // NOTE: this function is cursed by nll, we need to access the map at least twice to return
        //       a mutable reference to the node while we mutate the map
        // check if missing to add relations
        let idx = match self.node(&id) {
            Some(node) => node.idx,
            None => {
                // wee need to add the deps if we don't have the node or the resource is missing
                let idx = self.relations.add_node(id);

                self.add_relations(idx, deps);

                idx
            }
        };

        self.nodes
            .entry(id)
            .or_insert_with(|| Node::new(id, idx, resource))
    }

    pub(crate) fn get_or_insert(
        &mut self,
        id: Id,
        resource: NodeResource,
        deps: &[Id],
    ) -> &mut Node {
        self.insert(id, Some(resource), deps)
    }

    pub(crate) fn get_or_add_missing(&mut self, id: Id) -> &mut Node {
        self.insert(id, None, &[])
    }

    pub(crate) fn relations(&self) -> &StableDiGraph<Id, ()> {
        &self.relations
    }

    pub(crate) fn nodes(&self) -> &HashMap<Id, Node> {
        &self.nodes
    }

    fn add_relations(&mut self, idx: NodeIndex, deps: &[Id]) {
        debug_assert!(self.relations.contains_node(idx));

        for id in deps.iter().copied() {
            let dep_node = self.nodes.entry(id).or_insert_with(|| {
                let idx = self.relations.add_node(id);

                Node::new(id, idx, None)
            });
            // No recursive
            debug_assert_ne!(dep_node.idx, idx);

            // The index should always exists at this point
            debug_assert!(self.relations.contains_node(dep_node.idx));

            if self.relations.contains_edge(idx, dep_node.idx) {
                debug!("relation already exists {idx:?} -> {:?}", dep_node.idx);

                // no need to add the index, skipping
                continue;
            }

            self.relations.add_edge(idx, dep_node.idx, ());
        }
    }

    /// Returns a [`Vec`] of nodes that are only in the specified graph.
    ///
    /// It will filter the one that are related to another running deployment.
    pub(crate) fn nodes_only_in_deployment(
        &self,
        current: Id,
        start_idx: NodeIndex,
    ) -> Result<Vec<Id>, ServiceError> {
        debug_assert!(current.is_deployment());
        debug_assert_eq!(Some(current), self.get_id(start_idx).copied());

        petgraph::visit::DfsPostOrder::new(&self.relations, start_idx)
            .iter(&self.relations)
            .filter(|idx| {
                // filter the dependents deployment, and check that are not started
                let other_deployment = self.has_dependant_deployments(*idx, current);

                if other_deployment {
                    debug!(
                        "skipping {:?} which has another running deployment ",
                        self.get_id(*idx)
                    );
                }

                other_deployment
            })
            .map(|idx| {
                self.get_id(idx)
                    .copied()
                    .ok_or(ServiceError::MissingRelation)
            })
            .collect()
    }

    /// Check if the node is required by a running deployment other than the current one.
    fn has_dependant_deployments(&self, idx: NodeIndex, current: Id) -> bool {
        debug_assert!(current.is_deployment());
        self.dependent(idx)
            .filter_map(|id| {
                if current == *id {
                    None
                } else if id.is_deployment() {
                    let node = self.node(id);
                    debug_assert!(node.is_some());
                    node
                } else {
                    None
                }
            })
            .any(|deployment| deployment.is_up())
    }
}

impl Default for NodeGraph {
    fn default() -> Self {
        Self::new()
    }
}
