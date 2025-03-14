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
use edgehog_store::models::containers::device_mapping::DeviceMappingStatus;

use crate::properties::{device_mapping::AvailableDeviceMapping, AvailableProp, Client};

use super::{Context, Resource, Result};

#[derive(Debug, Default)]
pub(crate) struct DeviceMappingResource;

#[async_trait]
impl<D> Resource<D> for DeviceMappingResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        // TODO: check for missing device

        AvailableDeviceMapping::new(&ctx.id)
            .send(ctx.device, true)
            .await?;

        ctx.store
            .update_device_mapping_status(ctx.id, DeviceMappingStatus::Published)
            .await?;

        Ok(())
    }
}
