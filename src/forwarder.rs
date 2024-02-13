/*
 * This file is part of Edgehog.
 *
 * Copyright 2024 SECO Mind Srl
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
use edgehog_forwarder::astarte::{retrieve_connection_info, SessionInfo};
use edgehog_forwarder::connections_manager::ConnectionsManager;
use log::error;
use reqwest::Url;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::JoinHandle;

const CHANNEL_STATE_SIZE: usize = 5;

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
            Self::Connecting => write!(f, "connecting"),
            Self::Connected => write!(f, "connected"),
            Self::Disconnected => write!(f, "disconnected"),
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
        Self::String(value.status.to_string())
    }
}

/// Device forwarder.
///
/// It maintains a collection of tokio task handles, each one identified by a [`Key`] containing
/// the connection information and responsible for providing forwarder functionalities. For
/// instance, a task could open a remote terminal between the device and a certain host.
#[derive(Debug)]
pub struct Forwarder {
    tx_state: Sender<SessionState>,
    tasks: HashMap<Key, JoinHandle<()>>,
}

impl Forwarder {
    /// Initialize the forwarder instance, spawning also a task responsible for sending a property
    /// necessary to update the device session state.
    pub fn init<P>(publisher: P) -> Self
    where
        P: Publisher + 'static + Send + Sync,
    {
        let (tx_state, rx_state) = channel::<SessionState>(CHANNEL_STATE_SIZE);

        // the handle is not stored because it is ended when all tx_state are dropped (which is when
        // the Forwarder instance is dropped
        tokio::spawn(Self::handle_session_state_task(publisher, rx_state));

        Self {
            tx_state,
            tasks: Default::default(),
        }
    }

    async fn handle_session_state_task<P>(publisher: P, mut rx_state: Receiver<SessionState>)
    where
        P: Publisher,
    {
        while let Some(msg) = rx_state.recv().await {
            let ipath = format!("/{}/status", msg.token);
            let idata = msg.into();

            if let Err(err) = publisher
                .send(
                    "io.edgehog.devicemanager.ForwarderSessionsState",
                    &ipath,
                    idata,
                )
                .await
            {
                error!("publisher send error, {err}");
            }
        }
    }

    /// Start a device forwarder instance.
    pub fn handle_sessions(&mut self, astarte_event: AstarteDeviceDataEvent) {
        let Some(idata) = Self::retrieve_astarte_data(astarte_event) else {
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
        let tx_state = self.tx_state.clone();
        let session_token = cinfo.session_token.clone();
        self.get_running(cinfo).or_insert_with(||
            // spawn a new task responsible for handling the remote terminal operations
            Self::spawn_task(bridge_url, session_token, tx_state));
    }

    fn retrieve_astarte_data(
        astarte_event: AstarteDeviceDataEvent,
    ) -> Option<HashMap<String, AstarteType>> {
        if astarte_event.path != "/request" {
            error!("received data from an unknown path/interface: {astarte_event:?}");
            return None;
        }

        let Aggregation::Object(idata) = astarte_event.data else {
            error!("received wrong Aggregation data type");
            return None;
        };

        Some(idata)
    }

    /// Remove terminated sessions and return the searched one.
    fn get_running(&mut self, cinfo: SessionInfo) -> Entry<Key, JoinHandle<()>> {
        // remove all finished tasks
        self.tasks.retain(|_, jh| !jh.is_finished());

        self.tasks.entry(Key(cinfo))
    }

    /// Spawn the task responsible for handling a device forwarder instance.
    fn spawn_task(
        bridge_url: Url,
        session_token: String,
        tx_state: Sender<SessionState>,
    ) -> JoinHandle<()> {
        tokio::spawn(Self::handle_session(bridge_url, session_token, tx_state))
    }

    /// Handle remote session connection, operations and disconnection.
    async fn handle_session(
        bridge_url: Url,
        session_token: String,
        tx_state: Sender<SessionState>,
    ) {
        // update the session state to "connected"
        if let Err(err) = tx_state
            .send(SessionState::connecting(session_token.clone()))
            .await
        {
            error!("failed to change session state to connecting, {err}");
        }

        match ConnectionsManager::connect(bridge_url.clone()).await {
            Ok(mut con_manager) => {
                // update the session state to "connected"
                if let Err(err) = tx_state
                    .send(SessionState::connected(session_token.clone()))
                    .await
                {
                    error!("failed to change session state to connected, {err}");
                }

                // handle the connections
                if let Err(err) = con_manager.handle_connections().await {
                    error!("failed to handle connections, {err}");
                }

                // update the session state to "disconnected"
                if let Err(err) = tx_state
                    .send(SessionState::disconnected(session_token))
                    .await
                {
                    error!("failed to change session state to disconnected, {err}");
                }
            }
            Err(err) => {
                // update the session state to "disconnected"
                if let Err(err) = tx_state
                    .send(SessionState::disconnected(session_token))
                    .await
                {
                    error!("failed to change session state to disconnected, {err}");
                }
                error!("failed to connect, {err}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::tests::MockPublisher;
    use mockall::predicate;

    fn remote_terminal_req(session_token: &str, port: i32, host: &str) -> AstarteDeviceDataEvent {
        let mut data = HashMap::with_capacity(3);
        data.insert(
            "session_token".to_string(),
            AstarteType::String(session_token.to_string()),
        );
        data.insert("port".to_string(), AstarteType::Integer(port));
        data.insert("host".to_string(), AstarteType::String(host.to_string()));

        let data = Aggregation::Object(data);

        AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.RemoteTerminalRequest".to_string(),
            path: "/request".to_string(),
            data,
        }
    }

    #[tokio::test]
    async fn test_handle_session_state_task() {
        let mut mock_pub = MockPublisher::new();

        mock_pub
            .expect_send()
            .with(
                predicate::eq("io.edgehog.devicemanager.ForwarderSessionsState"),
                predicate::eq("/abcd/status"),
                predicate::eq(AstarteType::String("connecting".to_string())),
            )
            .times(1)
            .returning(|_, _, _| Ok(()));

        let (tx_state, rx_close) = channel::<SessionState>(CHANNEL_STATE_SIZE);

        let handle = tokio::spawn(Forwarder::handle_session_state_task(mock_pub, rx_close));

        // test sending and receiving

        tx_state
            .send(SessionState::connecting("abcd".to_string()))
            .await
            .unwrap();

        // when dropping the forwarder, the tx_state channel extremity is dropped too, therefore the
        // task handling session state must terminate.
        drop(tx_state);

        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_retrieve_astarte_data() {
        // wrong path
        let data_event = AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.RemoteTerminalRequest".to_string(),
            path: "/WRONG_PATH".to_string(),
            data: Aggregation::Individual(AstarteType::Boolean(false)),
        };

        assert_eq!(None, Forwarder::retrieve_astarte_data(data_event));

        // wrong aggregation data
        let data_event = AstarteDeviceDataEvent {
            interface: "io.edgehog.devicemanager.RemoteTerminalRequest".to_string(),
            path: "/request".to_string(),
            data: Aggregation::Individual(AstarteType::Boolean(false)),
        };

        assert_eq!(None, Forwarder::retrieve_astarte_data(data_event));

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

        assert_eq!(Some(data), Forwarder::retrieve_astarte_data(data_event))
    }
}
