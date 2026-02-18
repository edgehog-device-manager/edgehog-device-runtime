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

use edgehog_store::models::containers::image::ImageStatus;
use tracing::warn;

use crate::{
    image::{Image, ImageError},
    properties::{AvailableProp, Client, image::AvailableImage},
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone)]
pub(crate) struct ImageResource {
    pub(crate) image: Image,
}

impl ImageResource {
    pub(crate) fn new(image: Image) -> Self {
        Self { image }
    }
}

impl<D> Resource<D> for ImageResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id).send(ctx.device, false).await?;

        ctx.store
            .update_image_status(ctx.id, ImageStatus::Published)
            .await?;

        Ok(())
    }
}

impl<D> Create<D> for ImageResource
where
    D: Client + Send + Sync + 'static,
{
    async fn fetch(ctx: &mut Context<'_, D>) -> Result<(State, Self)> {
        let mut resource = ctx
            .store
            .find_image(ctx.id)
            .await?
            .ok_or(ResourceError::Missing {
                id: ctx.id,
                resource: "image",
            })?;

        let exists = resource.image.inspect(ctx.client).await?.is_some();

        if exists {
            ctx.store
                .update_image_local_id(ctx.id, resource.image.id.clone())
                .await?;

            Ok((State::Created, resource))
        } else {
            Ok((State::Missing, resource))
        }
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        self.image.pull(ctx.client).await?;

        ctx.store
            .update_image_local_id(ctx.id, self.image.id.clone())
            .await?;

        AvailableImage::new(&ctx.id).send(ctx.device, true).await?;

        ctx.store
            .update_image_status(ctx.id, ImageStatus::Pulled)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        match self.image.remove(ctx.client).await {
            Ok(_) => {}
            // HACK: this is a work around for two images having the same digest but different
            //       references.
            Err(ImageError::ImageInUse(err)) => {
                warn!(
                    error = format!("{:#}", eyre::Report::new(err)),
                    "image in use by another container"
                );
            }
            Err(err) => {
                return Err(err.into());
            }
        }

        Ok(())
    }

    async fn unset(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_image(ctx.id).await?;

        Ok(())
    }

    async fn refresh(ctx: &mut Context<'_, D>) -> Result<()> {
        let Some(mut resource) = ctx.store.find_image(ctx.id).await? else {
            warn!("couldn't find image");

            return Ok(());
        };

        let pulled = resource.image.inspect(ctx.client).await?.is_some();

        AvailableImage::new(&ctx.id)
            .send(ctx.device, pulled)
            .await?;

        Ok(())
    }
}
