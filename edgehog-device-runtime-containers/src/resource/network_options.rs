// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use edgehog_store::models::containers::network_options::NetworkOptionsStatus;

use crate::properties::network_options::{AvailableNetworkOptions, AvailableNetworkOptionsStatus};
use crate::properties::{AvailableProp, Client};

use super::{Context, Resource, Result};

#[derive(Debug, Default)]
pub(crate) struct NetworkOptionsResource;

impl<D> Resource<D> for NetworkOptionsResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableNetworkOptions::new(&ctx.id)
            .send(ctx.device, AvailableNetworkOptionsStatus::Present)
            .await?;
        ctx.store
            .update_network_options_status(ctx.id, NetworkOptionsStatus::Published)
            .await?;

        Ok(())
    }

    async fn unset(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableNetworkOptions::new(&ctx.id)
            .unset(ctx.device)
            .await?;

        ctx.store.delete_network_options(ctx.id).await?;

        Ok(())
    }
}
