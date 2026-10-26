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

//! Send events to Astarte

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::types::TypeError;
use astarte_device_sdk::{Client, astarte_device_error};
use bollard::models::EventMessageTypeEnum;
use eyre::Context;
use futures::TryStreamExt;
use tokio::sync::mpsc;
use tracing::{debug, error, instrument, trace};
use uuid::Uuid;

use crate::Docker;
use crate::event::ContainerRuntimeEvent;
use crate::service::{Id, ResourceType, events::ContainerEvent};
use crate::store::StateStore;

use self::container::ContainerEvent as DatastreamContainerEvent;
use self::image::ImageEvent;
use self::network::NetworkEvent;
use self::volume::VolumeEvent;

pub(crate) mod container;
pub(crate) mod deployment;
pub(crate) mod env_file;
pub(crate) mod file_bind;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

/// Sends a datastream of a container envet.
pub(crate) trait DatastreamEvent:
    TryInto<AstarteObject, Error = astarte_device_error::Error<TypeError>>
{
    const INTERFACE: &str;

    type Status;

    fn new(status: Self::Status, message: impl Into<String>, add_info: Vec<String>) -> Self;

    fn with_status(status: Self::Status) -> Self {
        Self::new(status, String::new(), Vec::new())
    }

    fn with_message(status: Self::Status, message: impl Into<String>) -> Self {
        Self::new(status, message, Vec::new())
    }

    fn with_error(status: Self::Status, error: &dyn std::error::Error) -> Self {
        let message = error.to_string();

        let add_info: Vec<String> = std::iter::successors(error.source(), |prev| prev.source())
            .map(|e| e.to_string())
            .collect();

        Self::new(status, message, add_info)
    }

    async fn send<D>(self, id: &Uuid, device: &mut D)
    where
        D: Client + Sync + 'static,
    {
        self.send_with_timestamp(id, device, None).await
    }

    #[instrument(skip_all, fields(%id))]
    async fn send_with_timestamp<D>(self, id: &Uuid, device: &mut D, ts: Option<DateTime<Utc>>)
    where
        D: Client + Sync + 'static,
    {
        let data: AstarteObject = match self.try_into() {
            Ok(data) => data,
            Err(error) => {
                error!(%error, interface = Self::INTERFACE, "couldn't convert to Astarte object");

                return;
            }
        };

        let ts = ts.unwrap_or_else(Utc::now);

        let res = device
            .send_object_with_timestamp(Self::INTERFACE, &format!("/{id}"), data, ts)
            .await;

        if let Err(error) = res {
            error!(%error, interface = Self::INTERFACE, "couldn't send event");
        }
    }
}

/// Handles the events received from the container runtime
#[derive(Debug)]
pub struct RuntimeListener<C> {
    device: C,
    client: Docker,
    store: StateStore,
    tx: mpsc::Sender<ContainerEvent>,
}

impl<C> RuntimeListener<C>
where
    C: Client + Sync + 'static,
{
    /// Creates a new instance.
    pub fn new(
        device: C,
        client: Docker,
        store: StateStore,
        tx: mpsc::Sender<ContainerEvent>,
    ) -> Self {
        Self {
            device,
            client,
            store,
            tx,
        }
    }

    /// Handles events of the container runtime and sends the telemetry to Astarte.
    #[instrument(skip(self))]
    pub async fn handle_events(&mut self) -> eyre::Result<()> {
        let mut stream = self.client.events();

        while let Some(event) = stream.try_next().await? {
            let Some(typ) = event.get_type() else {
                trace!("missing event type");

                continue;
            };
            let Some(local_id) = event.get_local_id() else {
                trace!("missing local id");

                continue;
            };

            let res = match typ {
                EventMessageTypeEnum::CONTAINER => self.handle_container(local_id, &event).await,
                EventMessageTypeEnum::IMAGE => self.handle_image(local_id, &event).await,
                EventMessageTypeEnum::NETWORK => self.handle_network(local_id, &event).await,
                EventMessageTypeEnum::VOLUME => self.handle_volume(local_id, &event).await,
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
            .await
            .wrap_err("couldn't send refresh event")?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_container(
        &mut self,
        local_id: &str,
        event: &ContainerRuntimeEvent,
    ) -> eyre::Result<()> {
        let Some(id) = self
            .store
            .find_container_by_local_id(local_id.to_string())
            .await?
        else {
            trace!("couldn't find container");

            return Ok(());
        };

        if let Some(status) = event.container_action() {
            DatastreamContainerEvent::with_status(status)
                .send_with_timestamp(&id, &mut self.device, event.timestamp())
                .await;
        }

        self.refresh(Id::new(ResourceType::Container, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_image(
        &mut self,
        local_id: &str,
        event: &ContainerRuntimeEvent,
    ) -> eyre::Result<()> {
        let Some((id, _)) = self
            .store
            .find_image_by_local_id(local_id.to_string())
            .await?
        else {
            trace!("couldn't find image");

            return Ok(());
        };

        if let Some(status) = event.image_action() {
            ImageEvent::with_status(status)
                .send_with_timestamp(&id, &mut self.device, event.timestamp())
                .await;
        }

        self.refresh(Id::new(ResourceType::Image, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_volume(
        &mut self,
        name: &str,
        event: &ContainerRuntimeEvent,
    ) -> eyre::Result<()> {
        let id = Uuid::try_parse(name).wrap_err("couldn't parse the uuid")?;

        if !self.store.check_volume_exists(id).await? {
            trace!("couldn't find volume");

            return Ok(());
        };

        if let Some(status) = event.volume_action() {
            VolumeEvent::with_status(status)
                .send_with_timestamp(&id, &mut self.device, event.timestamp())
                .await;
        }

        self.refresh(Id::new(ResourceType::Volume, id)).await?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle_network(
        &mut self,
        local_id: &str,
        event: &ContainerRuntimeEvent,
    ) -> eyre::Result<()> {
        let Some(id) = self
            .store
            .find_network_by_local_id(local_id.to_string())
            .await?
        else {
            trace!("couldn't find network");

            return Ok(());
        };

        if let Some(status) = event.network_action() {
            NetworkEvent::with_status(status)
                .send_with_timestamp(&id, &mut self.device, event.timestamp())
                .await;
        }

        self.refresh(Id::new(ResourceType::Network, id)).await?;

        Ok(())
    }
}
