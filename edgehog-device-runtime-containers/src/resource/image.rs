// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use async_trait::async_trait;
use edgehog_store::models::containers::image::ImageStatus;

use crate::{
    image::Image,
    properties::{image::AvailableImage, AvailableProp, Client},
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[async_trait]
impl<D> Resource<D> for Image
where
    D: Client + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id).send(ctx.device, false).await?;

        ctx.store
            .update_image_status(ctx.id, ImageStatus::Published)
            .await?;

        Ok(())
    }
}

#[async_trait]
impl<D> Create<D> for Image
where
    D: Client + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let image = ctx
            .store
            .image(ctx.id)
            .await?
            .ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "image",
            })?;

        let mut image = Image::from(image);

        let exists = image.inspect(ctx.client).await?.is_some();

        if exists {
            ctx.store
                .update_image_local_id(ctx.id, image.id.clone())
                .await?;

            Ok((State::Created, image))
        } else {
            Ok((State::Missing, image))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.inspect_or_create(ctx.client).await?;

        ctx.store
            .update_image_local_id(ctx.id, self.id.clone())
            .await?;

        AvailableImage::new(&ctx.id).send(ctx.device, true).await?;

        ctx.store
            .update_image_status(ctx.id, ImageStatus::Pulled)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.remove(ctx.client).await?;

        AvailableImage::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_image(ctx.id).await?;

        Ok(())
    }
}
