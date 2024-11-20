// This file is part of Astarte.
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

use std::fmt::{Debug, Display};

use crate::{container::Container, image::Image, network::Network, volume::Volume};

/// Resource and state.
#[derive(Debug, Clone)]
pub(crate) struct NodeResource {
    pub(crate) state: State,
    pub(crate) value: NodeType,
}

impl NodeResource {
    pub(crate) fn new(state: State, resource: NodeType) -> Self {
        Self {
            state,
            value: resource,
        }
    }

    pub(crate) fn with_default(resource: NodeType) -> Self {
        Self::new(State::default(), resource)
    }
}

/// State of the object for the request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum State {
    // The resource was received but was not published
    #[default]
    Received,
    // The resource was published and saved to the state file
    Published,
    // The resource was created
    Created,
    // The resource was started
    Up,
}

impl Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match self {
            State::Received => "Missing",
            State::Published => "Published",
            State::Created => "Created",
            State::Up => "Up",
        };

        write!(f, "{state}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeType {
    Image(Image<String>),
    Volume(Volume<String>),
    Network(Network<String>),
    Container(Container<String>),
    Deployment,
}

impl From<Image<String>> for NodeType {
    fn from(value: Image<String>) -> Self {
        Self::Image(value)
    }
}

impl From<Volume<String>> for NodeType {
    fn from(value: Volume<String>) -> Self {
        Self::Volume(value)
    }
}

impl From<Network<String>> for NodeType {
    fn from(value: Network<String>) -> Self {
        Self::Network(value)
    }
}

impl From<Container<String>> for NodeType {
    fn from(value: Container<String>) -> Self {
        Self::Container(value)
    }
}
