// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

//! Resource for the service.
//!
//! Handles and generalizes the operation on the various type of resources.

use std::future::Future;

use tracing::debug;
use uuid::Uuid;

use crate::{
    Docker,
    error::DockerError,
    properties::{Client, PropertyError},
    store::{StateStore, StoreError},
};

pub(crate) mod container;
pub(crate) mod deployment;
pub(crate) mod device_mapping;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

/// Error returned from a Resource operation.
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ResourceError {
    /// couldn't publish resource property
    Property(#[from] PropertyError),
    /// couldn't complete the store operation
    Store(#[from] StoreError),
    /// couldn't complete docker operation
    Docker(#[source] DockerError),
    /// couldn't fetch the {resource} with id {id}
    Missing {
        /// Id of the resource
        id: Uuid,
        /// Type of the resource
        resource: &'static str,
    },
}

#[derive(Debug)]
pub(crate) struct Context<'a, D> {
    /// Id of the resource
    pub(crate) id: Uuid,
    pub(crate) store: &'a mut StateStore,
    pub(crate) device: &'a mut D,
    pub(crate) client: &'a Docker,
}

/// State of the object for the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    // The resource was published and saved to the state file
    Missing,
    // The resource was created
    Created,
}

impl<T> From<T> for ResourceError
where
    T: Into<DockerError>,
{
    fn from(value: T) -> Self {
        ResourceError::Docker(value.into())
    }
}

type Result<T> = std::result::Result<T, ResourceError>;

pub(crate) trait Resource<D>: Sized
where
    D: Client + Sync + 'static,
{
    fn publish(ctx: Context<'_, D>) -> impl Future<Output = Result<()>> + Send;
}

pub(crate) trait Create<D>: Resource<D>
where
    D: Client + Send + Sync + 'static,
{
    fn fetch(ctx: &mut Context<'_, D>) -> impl Future<Output = Result<(State, Self)>> + Send;

    fn create(&mut self, ctx: &mut Context<'_, D>) -> impl Future<Output = Result<()>> + Send;

    fn delete(&mut self, ctx: &mut Context<'_, D>) -> impl Future<Output = Result<()>> + Send;

    fn unset(&mut self, ctx: &mut Context<'_, D>) -> impl Future<Output = Result<()>> + Send;

    fn refresh(ctx: &mut Context<'_, D>) -> impl Future<Output = Result<()>> + Send;

    async fn up(mut ctx: Context<'_, D>) -> Result<Self> {
        let (state, mut resource) = Self::fetch(&mut ctx).await?;

        match state {
            State::Missing => {
                debug!("resource missing, creating it");

                resource.create(&mut ctx).await?;
            }
            State::Created => {
                debug!("resource already created");
            }
        }

        Ok(resource)
    }

    async fn down(mut ctx: Context<'_, D>) -> Result<()> {
        let (state, mut resource) = Self::fetch(&mut ctx).await?;

        match state {
            State::Missing => {
                debug!("resource already missing");
            }
            State::Created => {
                debug!("resource found, deleting it");

                resource.delete(&mut ctx).await?
            }
        }

        resource.unset(&mut ctx).await?;

        Ok(())
    }
}
