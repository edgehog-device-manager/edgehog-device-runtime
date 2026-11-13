// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

use edgehog_store::models::containers::image::{ImageStatus, PullPolicy};
use tracing::warn;

use crate::{
    events::{
        DatastreamEvent,
        image::{ImageEvent, ImageEventStatus},
    },
    image::{Image, ImageError},
    properties::{
        AvailableProp, Client,
        image::{AvailableImage, AvailableImageStatus},
    },
};

use super::{Context, Create, Resource, ResourceError, Result, State};

#[derive(Debug, Clone)]
pub(crate) struct ImageResource {
    pub(crate) image: Image,
    pub(crate) pull_policy: PullPolicy,
}

impl ImageResource {
    pub(crate) fn new(image: Image, pull_policy: PullPolicy) -> Self {
        Self { image, pull_policy }
    }
}

impl<D> Resource<D> for ImageResource
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id)
            .send_status(ctx.device, AvailableImageStatus::Received)
            .await?;
        ctx.store
            .update_image_status(ctx.id, ImageStatus::Published)
            .await?;

        Self::fetch(ctx).await?;

        Ok(())
    }
    async fn unset(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_image(ctx.id).await?;

        Ok(())
    }
}

impl<D> Create<D> for ImageResource
where
    D: Client + Send + Sync + 'static,
{
    const RESOURCE_NAME: &str = "image";

    async fn fetch(ctx: &mut Context<'_, D>) -> Result<Option<(State, Self)>> {
        let Some(mut resource) = ctx.store.find_image(ctx.id).await? else {
            return Ok(None);
        };

        let image_inspect = resource
            .image
            .inspect(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        if let Some(inspect_image) = image_inspect {
            let image = AvailableImage::with_inspect(&ctx.id, inspect_image);

            image.send(ctx.device, AvailableImageStatus::Pulled).await?;
            ctx.store
                .update_image_local_id(ctx.id, resource.image.id.clone())
                .await?;

            match resource.pull_policy {
                PullPolicy::Always => Ok(Some((State::Recreatable, resource))),
                PullPolicy::IfNotPresent => Ok(Some((State::Existing, resource))),
            }
        } else {
            AvailableImage::new(&ctx.id)
                .send(ctx.device, AvailableImageStatus::Missing)
                .await?;

            Ok(Some((State::Missing, resource)))
        }
    }

    async fn on_error(ctx: &mut Context<'_, D>, error: &(dyn std::error::Error + Send + Sync)) {
        ImageEvent::with_error(ImageEventStatus::Error, error)
            .send(&ctx.id, ctx.device)
            .await;
    }

    async fn create(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id)
            .send_status(ctx.device, AvailableImageStatus::Pulling)
            .await?;

        let image_inspect = self
            .image
            .pull(ctx.client)
            .await
            .map_err(ResourceError::docker)?;

        if let Some(image_inspect) = image_inspect {
            let image = AvailableImage::with_inspect(&ctx.id, image_inspect);
            image.send(ctx.device, AvailableImageStatus::Pulled).await?;
        } else {
            return Err(ResourceError::Invalid {
                id: ctx.id,
                resource: "image",
                ctx: "couldn't inspect the image after pull",
            });
        };

        ctx.store
            .update_image_local_id(ctx.id, self.image.id.clone())
            .await?;

        ctx.store
            .update_image_status(ctx.id, ImageStatus::Pulled)
            .await?;

        Ok(())
    }

    async fn delete(&mut self, ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableImage::new(&ctx.id)
            .send_status(ctx.device, AvailableImageStatus::Deleting)
            .await?;
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
                return Err(ResourceError::docker(err));
            }
        }

        Ok(())
    }
}
