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

use async_trait::async_trait;
use edgehog_store::models::containers::network::NetworkStatus;

use crate::{
    network::Network,
    properties::{network::AvailableNetwork, AvailableProp, Client},
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone)]
pub(crate) struct NetworkResource {
    pub(crate) network: Network,
}

impl NetworkResource {
    pub(crate) fn new(network: Network) -> Self {
        Self { network }
    }
}

#[async_trait]
impl<D> Resource<D> for NetworkResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableNetwork::new(&ctx.id)
            .send(ctx.device, false)
            .await?;

        ctx.store
            .update_network_status(ctx.id, NetworkStatus::Published)
            .await?;

        Ok(())
    }
}

impl<D> Create<D> for NetworkResource
where
    D: Client + Send + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let mut resource = ctx
            .store
            .find_network(ctx.id)
            .await?
            .ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "network",
            })?;

        let exists = resource.network.inspect(ctx.client).await?.is_some();

        if exists {
            ctx.store
                .update_network_local_id(ctx.id, resource.network.id.id.clone())
                .await?;

            Ok((State::Created, resource))
        } else {
            Ok((State::Missing, resource))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.network.create(ctx.client).await?;

        ctx.store
            .update_network_local_id(ctx.id, self.network.id.id.clone())
            .await?;

        AvailableNetwork::new(&ctx.id)
            .send(ctx.device, true)
            .await?;

        ctx.store
            .update_network_status(ctx.id, NetworkStatus::Created)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.network.remove(ctx.client).await?;

        AvailableNetwork::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_network(ctx.id).await?;

        Ok(())
    }
}
