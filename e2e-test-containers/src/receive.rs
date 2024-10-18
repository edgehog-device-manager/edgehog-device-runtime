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

use astarte_device_sdk::{properties::PropAccess, AstarteType, Client, Value};
use color_eyre::eyre::{bail, Context, OptionExt};
use edgehog_containers::{service::Service, store::StateStore, Docker};
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

    loop {
        let event = device.recv().await?;

        match event.interface.as_str() {
            "io.edgehog.devicemanager.apps.CreateImageRequest"
            | "io.edgehog.devicemanager.apps.CreateVolumeRequest"
            | "io.edgehog.devicemanager.apps.CreateNetworkRequest"
            | "io.edgehog.devicemanager.apps.CreateContainerRequest"
            | "io.edgehog.devicemanager.apps.CreateReleaseRequest" => {
                service.on_event(event).await?;
            }
            "io.edgehog.devicemanager.apps.DeploymentCommand" => {
                let release_id = event
                    .path
                    .strip_prefix('/')
                    .and_then(|s| s.strip_suffix("/command"))
                    .ok_or_eyre("unrecognize endpoint for ApplicationCommand")?;

                let Value::Individual(AstarteType::String(cmd)) = event.data else {
                    bail!("Invalid interface event: {event:?}");
                };

                match cmd.as_str() {
                    "start" => {
                        service.start(release_id).await?;
                    }
                    "stop" => unimplemented!(),
                    _ => {
                        bail!("unrecognize ApplicationCommand {cmd}");
                    }
                }
            }
            _ => {
                error!("unrecognize interface {}", event.interface);
            }
        }
    }
}
