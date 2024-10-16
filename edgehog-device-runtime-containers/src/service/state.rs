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

use std::fmt::Debug;

use astarte_device_sdk::Client;
use tracing::{debug, error, info, instrument};

use crate::Docker;

use super::{resource::NodeType, Id, Result, ServiceError};

/// State of the object for the request.
#[derive(Debug, Clone, Default)]
pub(crate) enum State {
    #[default]
    Missing,
    Stored(NodeType),
    Created(NodeType),
    Up(NodeType),
}

impl State {
    #[instrument(skip_all)]
    pub(crate) async fn store<D, T>(&mut self, id: &Id, device: &D, node: T) -> Result<()>
    where
        D: Debug + Client + Sync,
        T: Into<NodeType> + Debug,
    {
        let node = node.into();

        match self {
            State::Missing => {
                node.store(id, device).await?;

                *self = State::Stored(node);

                debug!("node {id} stored");
            }
            State::Stored(stored) | State::Created(stored) | State::Up(stored) => {
                debug_assert_eq!(node, *stored);
                if node != *stored {
                    error!("received resource, differs from already stored one")
                }
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn create<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        match self {
            State::Missing => return Err(ServiceError::Create(id.to_string())),
            State::Stored(node) => {
                node.create(id, device, client).await?;

                self.map_into(State::Created)?;

                debug!("node {id} created");
            }
            State::Created(_) | State::Up(_) => {
                debug!("node already created");
            }
        }
        Ok(())
    }

    #[instrument(skip_all)]
    async fn start<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        match self {
            State::Missing | State::Stored(_) => Err(ServiceError::Start(id.to_string())),
            State::Created(node) => {
                node.start(id, device, client).await?;

                self.map_into(State::Up)?;

                debug!("node {id} started");

                Ok(())
            }
            State::Up(_) => {
                debug!("node already up");

                Ok(())
            }
        }
    }

    #[instrument(skip_all)]
    pub(super) async fn up<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        match &*self {
            State::Missing => return Err(ServiceError::Missing(id.to_string())),
            State::Stored(_) => {
                self.create(id, device, client).await?;
                self.start(id, device, client).await?;
            }
            State::Created(_) => {
                self.start(id, device, client).await?;
            }
            State::Up(_) => {
                debug!("node already up");
            }
        }

        info!("node {id} up");

        Ok(())
    }

    fn map_into<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(NodeType) -> State,
    {
        *self = match std::mem::take(self) {
            // It's safe to return the error on missing since the taken one is also missing
            State::Missing => return Err(ServiceError::BugMissing),
            State::Stored(node) | State::Created(node) | State::Up(node) => f(node),
        };

        Ok(())
    }

    /// Returns `true` if the state is [`Missing`].
    ///
    /// [`Missing`]: State::Missing
    #[must_use]
    pub(crate) fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// Returns `true` if the state is [`Up`].
    ///
    /// [`Up`]: State::Up
    #[must_use]
    pub(crate) fn is_up(&self) -> bool {
        matches!(self, Self::Up(..))
    }
}
