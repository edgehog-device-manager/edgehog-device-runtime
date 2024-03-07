/*
 * This file is part of Edgehog.
 *
 * Copyright 2023-2024 SECO Mind Srl
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
use std::fmt::{Display, Formatter};
use std::{
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
    ops::Deref,
};

use crate::data::Publisher;
use astarte_device_sdk::types::AstarteType;
use astarte_device_sdk::{Aggregation, AstarteDeviceDataEvent};
use edgehog_forwarder::astarte::{retrieve_connection_info, AstarteError, SessionInfo};
use edgehog_forwarder::connections_manager::ConnectionsManager;
use log::{error, info};
use reqwest::Url;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
struct Key(SessionInfo);

impl Deref for Key {
    type Target = SessionInfo;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<SessionInfo> for Key {
    fn borrow(&self) -> &SessionInfo {
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
        self.session_token.hash(state);
    }
}

#[derive(Debug, Clone, Copy)]
enum SessionStatus {
    Connecting,
    Connected,
    Disconnected,
}

impl Display for SessionStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Disconnected => write!(f, "Disconnected"),
        }
    }
}

struct SessionState {
    token: String,
    status: SessionStatus,
}

/// Struct representing the state of a remote session with a device
impl SessionState {
    fn connecting(token: String) -> Self {
        Self {
            token,
            status: SessionStatus::Connecting,
        }
    }

    fn connected(token: String) -> Self {
        Self {
            token,
            status: SessionStatus::Connected,
        }
    }

    fn disconnected(token: String) -> Self {
        Self {
            token,
            status: SessionStatus::Disconnected,
        }
    }
}

impl From<SessionState> for AstarteType {
    fn from(value: SessionState) -> Self {
        match value.status {
            SessionStatus::Connecting | SessionStatus::Connected => {
                Self::String(value.status.to_string())
            }
            SessionStatus::Disconnected => Self::Unset,
        }
    }
}

impl SessionState {
    async fn send<P>(self, publisher: &P)
    where
        P: Publisher,
    {
        let ipath = format!("/{}/status", self.token);
        let idata = self.into();

        if let Err(err) = publisher
            .send(
                "io.edgehog.devicemanager.ForwarderSessionState",
                &ipath,
                idata,
            )
            .await
        {
            error!("publisher send error, {err}");
        }
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
    pub fn handle_sessions<P>(&mut self, astarte_event: AstarteDeviceDataEvent, publisher: P)
    where
        P: Publisher + 'static + Send + Sync,
    {
        let idata = match Self::retrieve_astarte_data(astarte_event) {
            Ok(idata) => idata,
            Err(err) => {
                error!("{err}");
                return;
            }
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

        // check if the remote terminal task is already running. if not, spawn a new task and add it
        // to the collection
        let session_token = cinfo.session_token.clone();
        self.get_running(cinfo).or_insert_with(||
            // spawn a new task responsible for handling the remote terminal operations
            tokio::spawn(Self::handle_session(bridge_url, session_token, publisher)));
    }

    fn retrieve_astarte_data(
        astarte_event: AstarteDeviceDataEvent,
    ) -> Result<HashMap<String, AstarteType>, AstarteError> {
        if astarte_event.path != "/request" {
            return Err(AstarteError::WrongPath(astarte_event.path));
        }

        let Aggregation::Object(idata) = astarte_event.data else {
            return Err(AstarteError::WrongData);
        };

        Ok(idata)
    }

    /// Remove terminated sessions and return the searched one.
    fn get_running(&mut self, cinfo: SessionInfo) -> Entry<Key, JoinHandle<()>> {
        // remove all finished tasks
        self.tasks.retain(|_, jh| !jh.is_finished());

        self.tasks.entry(Key(cinfo))
    }

    /// Handle remote session connection, operations and disconnection.
    async fn handle_session<P>(bridge_url: Url, session_token: String, publisher: P)
    where
        P: Publisher + 'static + Send + Sync,
    {
        // update the session state to "Connecting"
        SessionState::connecting(session_token.clone())
            .send(&publisher)
            .await;

        let mut con_manager = match ConnectionsManager::connect(bridge_url.clone()).await {
            Ok(con_manager) => con_manager,
            Err(err) => {
                // unset the session state, meaning that the device correctly disconnected itself
                SessionState::disconnected(session_token.clone())
                    .send(&publisher)
                    .await;

                error!("failed to connect, {err}");
                return;
            }
        };

        // update the session state to "Connected"
        SessionState::connected(session_token.clone())
            .send(&publisher)
            .await;

        // handle the connections
        if let Err(err) = con_manager.handle_connections().await {
            error!("failed to handle connections, {err}");
        }

        // unset the session state, meaning that the device correctly disconnected itself
        SessionState::disconnected(session_token.clone())
            .send(&publisher)
            .await;
        info!("forwarder correctly disconnected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_terminal_req(session_token: &str, port: i32, host: &str) -> AstarteDeviceDataEvent {
        let mut data = HashMap::with_capacity(3);
        data.insert(
            "session_token".to_string(),
            AstarteType::String(session_token.to_string()),
        );
        data.insert("port".to_string(), AstarteType::Integer(port));
        data.insert("host".to_string(), AstarteType::String(host.to_string()));
        data.insert("secure".to_string(), AstarteType::Boolean(false));

        let data = Aggregation::Object(data);

        AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/request".to_string(),
            data,
        }
    }

    #[tokio::test]
    async fn test_retrieve_astarte_data() {
        // wrong path
        let data_event = AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/WRONG_PATH".to_string(),
            data: Aggregation::Individual(AstarteType::Boolean(false)),
        };

        assert!(Forwarder::retrieve_astarte_data(data_event).is_err());

        // wrong aggregation data
        let data_event = AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/request".to_string(),
            data: Aggregation::Individual(AstarteType::Boolean(false)),
        };

        assert!(Forwarder::retrieve_astarte_data(data_event).is_err());

        // correct data event
        let data_event = remote_terminal_req("abcd", 8080, "127.0.0.1");

        let mut data = HashMap::with_capacity(3);
        data.insert(
            "session_token".to_string(),
            AstarteType::String("abcd".to_string()),
        );
        data.insert("port".to_string(), AstarteType::Integer(8080));
        data.insert(
            "host".to_string(),
            AstarteType::String("127.0.0.1".to_string()),
        );
        data.insert("secure".to_string(), AstarteType::Boolean(false));

        let res =
            Forwarder::retrieve_astarte_data(data_event).expect("failed to retrieve astarte data");

        assert_eq!(data, res)
    }
}
