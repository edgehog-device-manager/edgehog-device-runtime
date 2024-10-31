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

use std::path::Path;

use astarte_device_sdk::Client;
use async_trait::async_trait;
use edgehog_containers::{
    requests::ContainerRequest,
    service::{Service, ServiceError},
    store::StateStore,
    Docker,
};

use crate::controller::actor::Actor;

#[derive(Debug)]
pub(crate) struct ContainerService<D> {
    service: Service<D>,
}

impl<D> ContainerService<D> {
    pub(crate) async fn new(store_dir: &Path, device: D) -> Result<Self, ServiceError> {
        let client = Docker::connect().await?;
        let store = StateStore::open(store_dir.join("containers/state.json")).await?;

        let service = Service::new(client, store, device);

        Ok(Self { service })
    }
}

#[async_trait]
impl<D> Actor for ContainerService<D>
where
    D: Client + Send + Sync + 'static,
{
    type Msg = Box<ContainerRequest>;

    fn task() -> &'static str {
        "containers"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        self.service.init().await?;

        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        self.service.on_event(*msg).await?;

        Ok(())
    }
}
