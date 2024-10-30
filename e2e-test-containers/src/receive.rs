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

use std::{fmt::Debug, path::Path};

use astarte_device_sdk::{properties::PropAccess, Client, FromEvent};
use color_eyre::eyre::Context;
use edgehog_containers::{requests::ContainerRequest, service::Service, store::StateStore, Docker};
use tracing::error;

pub async fn receive<D>(device: D, store: &Path) -> color_eyre::Result<()>
where
    D: Debug + Client + Clone + PropAccess + Sync + 'static,
{
    let client = Docker::connect().await?;
    let store = StateStore::open(store.join("state.json"))
        .await
        .wrap_err("couldn't open the state store")?;

    let mut service = Service::new(client, store, device.clone());
    service.init().await?;

    loop {
        let event = device.recv().await?;

        match ContainerRequest::from_event(event) {
            Ok(req) => {
                service.on_event(req).await?;
            }
            Err(err) => {
                error!(error = %color_eyre::Report::new(err),"couldn't parse the event");
            }
        }
    }
}
