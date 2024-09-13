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

use std::collections::{hash_map::Entry, HashMap};
use std::fmt::{Display, Formatter};

use crate::data::Publisher;
use astarte_device_sdk::types::AstarteType;
use edgehog_forwarder::astarte::SessionInfo;
use edgehog_forwarder::connections_manager::{ConnectionsManager, Disconnected};
use log::{debug, error, info};
use reqwest::Url;
use tokio::task::JoinHandle;

const FORWARDER_SESSION_STATE_INTERFACE: &str = "io.edgehog.devicemanager.ForwarderSessionState";

/// Forwarder errors
#[derive(displaydoc::Display, thiserror::Error, Debug)]
pub enum ForwarderError {
    /// Astarte error
    Astarte(#[from] astarte_device_sdk::Error),

    /// Astarte type conversion error
    Type(#[from] astarte_device_sdk::types::TypeError),

    /// Connections manager error
    ConnectionsManager(#[from] edgehog_forwarder::connections_manager::Error),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl SessionState {
    /// Send a property to Astarte to update the session state.
    async fn send<P>(self, publisher: &P) -> Result<(), astarte_device_sdk::Error>
    where
        P: Publisher + 'static + Send + Sync,
    {
        let ipath = format!("/{}/status", self.token);

        match self.status {
            SessionStatus::Connecting | SessionStatus::Connected => {
                let idata = AstarteType::String(self.status.to_string());

                publisher
                    .send(FORWARDER_SESSION_STATE_INTERFACE, &ipath, idata)
                    .await
            }
            SessionStatus::Disconnected => {
                publisher
                    .unset(FORWARDER_SESSION_STATE_INTERFACE, &ipath)
                    .await
            }
        }
    }
}

/// Device forwarder.
///
/// It maintains a collection of tokio task handles, each one identified by a [`Key`] containing
/// the connection information and responsible for providing forwarder functionalities. For
/// instance, a task could open a remote terminal between the device and a certain host.
#[derive(Debug)]
pub struct Forwarder<P> {
    publisher: P,
    tasks: HashMap<SessionInfo, JoinHandle<()>>,
}

impl<P> Forwarder<P> {
    pub async fn init(publisher: P) -> Result<Self, ForwarderError>
    where
        P: Publisher + 'static + Send + Sync,
    {
        // unset all the existing sessions
        debug!("unsetting ForwarderSessionState property");
        for prop in publisher
            .interface_props(FORWARDER_SESSION_STATE_INTERFACE)
            .await?
        {
            debug!("unset {}", &prop.path);
            publisher
                .unset(FORWARDER_SESSION_STATE_INTERFACE, &prop.path)
                .await?;
        }

        Ok(Self {
            publisher,
            tasks: HashMap::default(),
        })
    }

    /// Start a device forwarder instance.
    pub fn handle_sessions(&mut self, sinfo: SessionInfo)
    where
        P: Publisher + Clone + 'static + Send + Sync,
    {
        let edgehog_url = match Url::try_from(&sinfo) {
            Ok(url) => url,
            Err(err) => {
                error!("invalid url, {err}");
                return;
            }
        };

        // check if the remote terminal task is already running. if not, spawn a new task and add it
        // to the collection
        // flag indicating whether the connection should use TLS, i.e. 'ws' or 'wss' scheme.
        let secure = sinfo.secure;
        let session_token = sinfo.session_token.clone();
        let publisher = self.publisher.clone();
        self.get_running(sinfo).or_insert_with(|| {
            info!("opening a new session");
            // spawn a new task responsible for handling the remote terminal operations
            tokio::spawn(async move {
                if let Err(err) =
                    Self::handle_session(edgehog_url, session_token, secure, publisher).await
                {
                    error!("session failed, {err}");
                }
            })
        });
    }

    /// Remove terminated sessions and return the searched one.
    fn get_running(&mut self, sinfo: SessionInfo) -> Entry<SessionInfo, JoinHandle<()>> {
        // remove all finished tasks
        self.tasks.retain(|_, jh| !jh.is_finished());

        self.tasks.entry(sinfo)
    }

    /// Handle remote session connection, operations and disconnection.
    async fn handle_session(
        edgehog_url: Url,
        session_token: String,
        secure: bool,
        publisher: P,
    ) -> Result<(), ForwarderError>
    where
        P: Publisher + 'static + Send + Sync,
    {
        // update the session state to "Connecting"
        SessionState::connecting(session_token.clone())
            .send(&publisher)
            .await?;

        if let Err(err) =
            Self::connect(edgehog_url, session_token.clone(), secure, &publisher).await
        {
            error!("failed to connect, {err}");
        }

        // unset the session state, meaning that the device correctly disconnected itself
        SessionState::disconnected(session_token.clone())
            .send(&publisher)
            .await?;

        info!("forwarder correctly disconnected");

        Ok(())
    }

    async fn connect(
        edgehog_url: Url,
        session_token: String,
        secure: bool,
        publisher: &P,
    ) -> Result<(), ForwarderError>
    where
        P: Publisher + 'static + Send + Sync,
    {
        let mut con_manager = ConnectionsManager::connect(edgehog_url.clone(), secure).await?;

        // update the session state to "Connected"
        SessionState::connected(session_token.clone())
            .send(publisher)
            .await?;

        // handle the connections
        while let Err(Disconnected(err)) = con_manager.handle_connections().await {
            error!("WebSocket disconnected, {err}");

            // in case of a websocket error, the connection has been lost, so update the session
            // state to "Connecting"
            SessionState::connecting(session_token.clone())
                .send(publisher)
                .await?;

            con_manager
                .reconnect()
                .await
                .map_err(ForwarderError::ConnectionsManager)?;

            // update the session state to "Connected" since connection has been re-established
            SessionState::connected(session_token.clone())
                .send(publisher)
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::data::tests::MockPubSub;
    use astarte_device_sdk::store::StoredProp;
    use astarte_device_sdk::{interface::def::Ownership, Value};
    use astarte_device_sdk::{DeviceEvent, FromEvent};
    use std::net::Ipv4Addr;

    #[test]
    fn test_session_status() {
        let sstatus = [
            SessionStatus::Connected,
            SessionStatus::Connecting,
            SessionStatus::Disconnected,
        ]
        .map(|ss| ss.to_string());
        let exp_res = ["Connected", "Connecting", "Disconnected"];

        // test display
        for (idx, el) in sstatus.into_iter().enumerate() {
            assert_eq!(&el, exp_res.get(idx).unwrap())
        }
    }

    #[test]
    fn test_session_state() {
        let sstates = [
            SessionState::connected("abcd".to_string()),
            SessionState::connecting("abcd".to_string()),
            SessionState::disconnected("abcd".to_string()),
        ];
        let exp_res = [
            SessionState {
                token: "abcd".to_string(),
                status: SessionStatus::Connected,
            },
            SessionState {
                token: "abcd".to_string(),
                status: SessionStatus::Connecting,
            },
            SessionState {
                token: "abcd".to_string(),
                status: SessionStatus::Disconnected,
            },
        ];

        for (idx, el) in sstates.into_iter().enumerate() {
            assert_eq!(&el, exp_res.get(idx).unwrap())
        }
    }

    #[tokio::test]
    async fn test_session_state_send() {
        let ss = SessionState::connected("abcd".to_string());
        let mut pub_sub = MockPubSub::new();

        pub_sub
            .expect_send()
            .withf(move |iface, ipath, idata| {
                iface == FORWARDER_SESSION_STATE_INTERFACE
                    && ipath == "/abcd/status"
                    && idata == &AstarteType::String("Connected".to_string())
            })
            .returning(|_, _, _| Ok(()));

        pub_sub
            .expect_unset()
            .withf(move |iface, ipath| {
                iface == FORWARDER_SESSION_STATE_INTERFACE && ipath == "/abcd/status"
            })
            .returning(|_, _| Ok(()));

        let res = ss.send(&pub_sub).await;

        assert!(res.is_ok());

        // send unset in case of SessionState::disconnected
        let ss = SessionState::disconnected("abcd".to_string());
        let res = ss.send(&pub_sub).await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_init_forwarder() {
        let mut pub_sub = MockPubSub::new();
        mock_forwarder_init(&mut pub_sub);
        let f = Forwarder::init(pub_sub).await;

        assert!(f.is_ok());

        // test when an error is returned by the publisher
        let mut pub_sub = MockPubSub::new();

        pub_sub
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                // the returned error is irrelevant, it is only necessary to the test
                Err(astarte_device_sdk::error::Error::ConnectionTimeout)
            });

        let f = Forwarder::init(pub_sub).await;

        assert!(f.is_err());

        let mut pub_sub = MockPubSub::new();

        pub_sub
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                Ok(vec![StoredProp {
                    interface: FORWARDER_SESSION_STATE_INTERFACE.to_string(),
                    path: "/abcd/status".to_string(),
                    value: AstarteType::String("Connected".to_string()),
                    interface_major: 0,
                    ownership: Ownership::Device,
                }])
            });

        pub_sub
            .expect_unset()
            .withf(move |iface, ipath| {
                iface == "io.edgehog.devicemanager.ForwarderSessionState" && ipath == "/abcd/status"
            })
            // the returned error is irrelevant, it is only necessary to the test
            .returning(|_, _| Err(astarte_device_sdk::error::Error::ConnectionTimeout));

        let f = Forwarder::init(pub_sub).await;

        assert!(f.is_err());
    }

    fn mock_forwarder_init(pub_sub: &mut MockPubSub) {
        pub_sub
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                Ok(vec![StoredProp {
                    interface: FORWARDER_SESSION_STATE_INTERFACE.to_string(),
                    path: "/abcd/status".to_string(),
                    value: AstarteType::String("Connected".to_string()),
                    interface_major: 0,
                    ownership: Ownership::Device,
                }])
            });

        pub_sub
            .expect_unset()
            .withf(move |iface, ipath| {
                iface == "io.edgehog.devicemanager.ForwarderSessionState" && ipath == "/abcd/status"
            })
            .returning(|_, _| Ok(()));
    }

    #[tokio::test]
    async fn test_handle_sessions() {
        let mut publisher = MockPubSub::new();

        publisher.expect_clone().returning(MockPubSub::new);

        let mut f = Forwarder {
            publisher,
            tasks: HashMap::from([(
                SessionInfo {
                    host: Ipv4Addr::LOCALHOST.to_string(),
                    port: 8080,
                    session_token: "abcd".to_string(),
                    secure: false,
                },
                tokio::spawn(async {}),
            )]),
        };

        let astarte_event = DeviceEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object(HashMap::from([
                (
                    "host".to_string(),
                    AstarteType::String("127.0.0.1".to_string()),
                ),
                ("port".to_string(), AstarteType::Integer(8080)),
                (
                    "session_token".to_string(),
                    AstarteType::String("abcd".to_string()),
                ),
                ("secure".to_string(), AstarteType::Boolean(false)),
            ])),
        };

        let session = SessionInfo::from_event(astarte_event).unwrap();

        // the test is successful once handle_sessions terminates
        f.handle_sessions(session);
    }
}
