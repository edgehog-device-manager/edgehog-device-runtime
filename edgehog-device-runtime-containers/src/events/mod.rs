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

//! Send events to Astarte

use bollard::models::EventMessageTypeEnum;
use eyre::Context;
use futures::{future, TryStreamExt};
use tokio::sync::mpsc;
use tracing::{debug, error, instrument};
use uuid::Uuid;

use crate::{
    service::{events::ContainerEvent, Id, ResourceType},
    store::StateStore,
    Docker,
};

pub(crate) mod deployment;

/// Handles the events received from the container runtime
#[derive(Debug)]
pub struct RuntimeListener {
    client: Docker,
    store: StateStore,
    tx: mpsc::UnboundedSender<ContainerEvent>,
}

impl RuntimeListener {
    /// Creates a new instance.
    pub fn new(
        client: Docker,
        store: StateStore,
        tx: mpsc::UnboundedSender<ContainerEvent>,
    ) -> Self {
        Self { client, store, tx }
    }

    /// Handles events of the container runtime and sends the telemetry to Astarte.
    #[instrument(skip(self))]
    pub async fn handle_events(&mut self) -> eyre::Result<()> {
        let mut stream = self.client.events().try_filter_map(|event| {
            let type_id = event.typ.zip(event.actor.and_then(|actor| actor.id));

            future::ok(type_id)
        });

        while let Some((typ, local_id)) = stream.try_next().await? {
            let res = match typ {
                EventMessageTypeEnum::CONTAINER => self.handle_container(local_id).await,
                EventMessageTypeEnum::IMAGE => self.handle_image(local_id).await,
                EventMessageTypeEnum::NETWORK => self.handle_network(local_id).await,
                EventMessageTypeEnum::VOLUME => self.handle_volume(local_id).await,
                _ => {
                    debug!(%typ, "skipping event");

                    continue;
                }
            };

            if let Err(err) = res {
                error!(
                    error = format!("{err:#}"),
                    "couldn't handle container event"
                );
            }
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn refresh(&self, id: Id) -> eyre::Result<()> {
        self.tx
            .send(ContainerEvent::Refresh(id))
            .wrap_err("couldn't send refresh event")?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_container(&mut self, local_id: String) -> eyre::Result<()> {
        let Some(id) = self
            .store
            .find_container_by_local_id(local_id.clone())
            .await?
        else {
            debug!("couldn't find container");

            return Ok(());
        };

        self.refresh(Id::new(ResourceType::Container, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_image(&mut self, local_id: String) -> eyre::Result<()> {
        let Some((id, _)) = self.store.find_image_by_local_id(local_id.clone()).await? else {
            debug!("couldn't find image");

            return Ok(());
        };

        self.refresh(Id::new(ResourceType::Image, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_volume(&mut self, name: String) -> eyre::Result<()> {
        let id = Uuid::try_parse(&name).wrap_err("couldn't parse the uuid")?;

        if !self.store.check_volume_exists(id).await? {
            debug!("couldn't find volume");

            return Ok(());
        };

        self.refresh(Id::new(ResourceType::Volume, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_network(&mut self, local_id: String) -> eyre::Result<()> {
        let Some(id) = self
            .store
            .find_network_by_local_id(local_id.clone())
            .await?
        else {
            debug!("couldn't find network");

            return Ok(());
        };

        self.refresh(Id::new(ResourceType::Network, id)).await?;

        Ok(())
    }
}
