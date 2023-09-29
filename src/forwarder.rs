/*
 * This file is part of Edgehog.
 *
 * Copyright 2023 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

//! Manage the device forwarder operation.

use std::borrow::Borrow;
use std::{
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
    ops::Deref,
};

use astarte_device_sdk::{Aggregation, AstarteDeviceDataEvent};
use edgehog_forwarder::astarte::{retrieve_connection_info, ConnectionInfo};
use edgehog_forwarder::connections_manager::ConnectionsManager;
use log::error;
use reqwest::Url;
use tokio::task::JoinHandle;

#[derive(Debug)]
struct Key(ConnectionInfo);

impl Deref for Key {
    type Target = ConnectionInfo;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<ConnectionInfo> for Key {
    fn borrow(&self) -> &ConnectionInfo {
        &self.0
    }
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.host == other.host && self.port == other.port
    }
}

impl Eq for Key {}

impl Hash for Key {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.host.hash(state);
        self.port.hash(state);
    }
}

/// Device forwarder.
///
/// It maintains a collection of tokio task handles, each one identified by a [`Key`] containing
/// the connection information and responsible for providing forwarder functionalities. For
/// instance, a task could open a remote terminal between the device and a certain host.
#[derive(Debug, Default)]
pub struct Forwarder {
    tasks: HashMap<Key, JoinHandle<()>>,
}

impl Forwarder {
    /// Start a device forwarder instance.
    pub async fn start(&mut self, astarte_event: AstarteDeviceDataEvent) {
        if astarte_event.path != "/request" {
            error!("received data from an unknown path/interface: {astarte_event:?}");
            return;
        }

        let Aggregation::Object(idata) = astarte_event.data else {
            error!("received wrong Aggregation data type");
            return;
        };

        // retrieve the Url that the device must use to open a WebSocket connection with a host
        let cinfo = match retrieve_connection_info(idata) {
            Ok(cinfo) => cinfo,
            // error while retrieving the connection information from the Astarte data
            Err(err) => {
                error!("{err}");
                return;
            }
        };

        let bridge_url = match Url::try_from(&cinfo) {
            Ok(url) => url,
            Err(err) => {
                error!("invalid url, {err}");
                return;
            }
        };

        // check if the remote terminal task is already running. if not, spawn a new task and add it to the collection
        self.get_running(cinfo).or_insert_with(||
            // spawn a new task responsible for handling the remote terminal operations
            Self::spawn_task(bridge_url));
    }

    fn get_running(&mut self, cinfo: ConnectionInfo) -> Entry<Key, JoinHandle<()>> {
        // remove all finished tasks
        self.tasks.retain(|_, jh| !jh.is_finished());

        self.tasks.entry(Key(cinfo))
    }

    /// Task handling a device forwarder instance.
    fn spawn_task(bridge_url: Url) -> JoinHandle<()> {
        tokio::spawn(async move {
            match ConnectionsManager::connect(bridge_url).await {
                Ok(mut con_manager) => {
                    // handle the connections
                    if let Err(err) = con_manager.handle_connections().await {
                        error!("failed to handle connections, {err}");
                    }
                }
                Err(err) => error!("failed to connect, {err}"),
            }
        })
    }
}
