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

use std::collections::{HashMap, hash_map::Entry};
use std::fmt::{Display, Formatter};

use astarte_device_sdk::prelude::PropAccess;
use astarte_device_sdk::types::AstarteData;
use edgehog_forwarder::astarte::SessionInfo;
use edgehog_forwarder::connections_manager::{ConnectionsManager, Disconnected};
use reqwest::Url;
use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use crate::Client;

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

impl From<SessionState> for Option<AstarteData> {
    fn from(value: SessionState) -> Self {
        match value.status {
            SessionStatus::Connecting | SessionStatus::Connected => {
                Some(AstarteData::String(value.status.to_string()))
            }
            SessionStatus::Disconnected => None,
        }
    }
}

impl SessionState {
    /// Send a property to Astarte to update the session state.
    async fn send<C>(self, client: &mut C) -> Result<(), astarte_device_sdk::Error>
    where
        C: Client + Send + Sync + 'static,
    {
        let ipath = format!("/{}/status", self.token);
        let idata: Option<AstarteData> = self.into();

        match idata {
            Some(idata) => {
                client
                    .set_property(FORWARDER_SESSION_STATE_INTERFACE, &ipath, idata)
                    .await
            }
            None => {
                client
                    .unset_property(FORWARDER_SESSION_STATE_INTERFACE, &ipath)
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
pub struct Forwarder<C> {
    client: C,
    tasks: HashMap<SessionInfo, JoinHandle<()>>,
}

impl<C> Forwarder<C> {
    pub async fn init(mut client: C) -> Result<Self, ForwarderError>
    where
        C: Client + PropAccess + Send + Sync + 'static,
    {
        // unset all the existing sessions
        debug!("unsetting ForwarderSessionState property");
        for prop in client
            .interface_props(FORWARDER_SESSION_STATE_INTERFACE)
            .await?
        {
            debug!("unset {}", &prop.path);
            client
                .unset_property(FORWARDER_SESSION_STATE_INTERFACE, &prop.path)
                .await?;
        }

        Ok(Self {
            client,
            tasks: HashMap::default(),
        })
    }

    /// Start a device forwarder instance.
    pub fn handle_sessions(&mut self, sinfo: SessionInfo)
    where
        C: Client + 'static + Send + Sync,
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
        let publisher = self.client.clone();
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
    fn get_running(&mut self, sinfo: SessionInfo) -> Entry<'_, SessionInfo, JoinHandle<()>> {
        // remove all finished tasks
        self.tasks.retain(|_, jh| !jh.is_finished());

        self.tasks.entry(sinfo)
    }

    /// Handle remote session connection, operations and disconnection.
    async fn handle_session(
        edgehog_url: Url,
        session_token: String,
        secure: bool,
        mut client: C,
    ) -> Result<(), ForwarderError>
    where
        C: Client + Send + Sync + 'static,
    {
        // update the session state to "Connecting"
        SessionState::connecting(session_token.clone())
            .send(&mut client)
            .await?;

        if let Err(err) =
            Self::connect(edgehog_url, session_token.clone(), secure, &mut client).await
        {
            error!("failed to connect, {err}");
        }

        // unset the session state, meaning that the device correctly disconnected itself
        SessionState::disconnected(session_token.clone())
            .send(&mut client)
            .await?;

        info!("forwarder correctly disconnected");

        Ok(())
    }

    async fn connect(
        edgehog_url: Url,
        session_token: String,
        secure: bool,
        client: &mut C,
    ) -> Result<(), ForwarderError>
    where
        C: Client + Send + Sync + 'static,
    {
        let mut con_manager = ConnectionsManager::connect(edgehog_url.clone(), secure).await?;

        // update the session state to "Connected"
        SessionState::connected(session_token.clone())
            .send(client)
            .await?;

        // handle the connections
        while let Err(Disconnected(err)) = con_manager.handle_connections().await {
            error!("WebSocket disconnected, {err}");

            // in case of a websocket error, the connection has been lost, so update the session
            // state to "Connecting"
            SessionState::connecting(session_token.clone())
                .send(client)
                .await?;

            con_manager
                .reconnect()
                .await
                .map_err(ForwarderError::ConnectionsManager)?;

            // update the session state to "Connected" since connection has been re-established
            SessionState::connected(session_token.clone())
                .send(client)
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use astarte_device_sdk::Value;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::astarte_interfaces::schema::Ownership;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::store::{SqliteStore, StoredProp};
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::{DeviceEvent, FromEvent};
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};
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

    #[test]
    fn test_astarte_type_from_session_state() {
        let sstates = [
            SessionState::connected("abcd".to_string()),
            SessionState::connecting("abcd".to_string()),
            SessionState::disconnected("abcd".to_string()),
        ]
        .map(Option::<AstarteData>::from);
        let exp_res = [
            Some(AstarteData::String("Connected".to_string())),
            Some(AstarteData::String("Connecting".to_string())),
            None,
        ];

        for (el, exp) in sstates.into_iter().zip(exp_res) {
            assert_eq!(el, exp);
        }
    }

    #[tokio::test]
    async fn test_session_state_send() {
        let ss = SessionState::connected("abcd".to_string());
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .with(
                predicate::eq(FORWARDER_SESSION_STATE_INTERFACE),
                predicate::eq("/abcd/status"),
                predicate::eq(AstarteData::from("Connected")),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));

        client
            .expect_unset_property()
            .with(
                predicate::eq(FORWARDER_SESSION_STATE_INTERFACE),
                predicate::eq("/abcd/status"),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _| Ok(()));

        let res = ss.send(&mut client).await;

        assert!(res.is_ok());

        // send unset in case of SessionState::disconnected
        let ss = SessionState::disconnected("abcd".to_string());
        let res = ss.send(&mut client).await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_init_forwarder() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        mock_forwarder_init(&mut client);
        let f = Forwarder::init(client).await;

        assert!(f.is_ok());

        // test when an error is returned by the publisher
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                // the returned error is irrelevant, it is only necessary to the test
                Err(astarte_device_sdk::error::Error::ConnectionTimeout)
            });

        let f = Forwarder::init(client).await;

        assert!(f.is_err());

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                Ok(vec![StoredProp {
                    interface: FORWARDER_SESSION_STATE_INTERFACE.to_string(),
                    path: "/abcd/status".to_string(),
                    value: AstarteData::String("Connected".to_string()),
                    interface_major: 0,
                    ownership: Ownership::Device,
                }])
            });

        client
            .expect_unset_property()
            .withf(move |iface, ipath| {
                iface == "io.edgehog.devicemanager.ForwarderSessionState" && ipath == "/abcd/status"
            })
            // the returned error is irrelevant, it is only necessary to the test
            .returning(|_, _| Err(astarte_device_sdk::error::Error::ConnectionTimeout));

        let f = Forwarder::init(client).await;

        assert!(f.is_err());
    }

    fn mock_forwarder_init(pub_sub: &mut MockDeviceClient<Mqtt<SqliteStore>>) {
        pub_sub
            .expect_interface_props()
            .withf(move |iface: &str| iface == FORWARDER_SESSION_STATE_INTERFACE)
            .returning(|_: &str| {
                Ok(vec![StoredProp {
                    interface: FORWARDER_SESSION_STATE_INTERFACE.to_string(),
                    path: "/abcd/status".to_string(),
                    value: AstarteData::String("Connected".to_string()),
                    interface_major: 0,
                    ownership: Ownership::Device,
                }])
            });

        pub_sub
            .expect_unset_property()
            .withf(move |iface, ipath| {
                iface == "io.edgehog.devicemanager.ForwarderSessionState" && ipath == "/abcd/status"
            })
            .returning(|_, _| Ok(()));
    }

    #[tokio::test]
    async fn test_handle_sessions() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_clone()
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let mut f = Forwarder {
            client,
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

        let data = AstarteObject::from_iter([
            (
                "host".to_string(),
                AstarteData::String("127.0.0.1".to_string()),
            ),
            ("port".to_string(), AstarteData::Integer(8080)),
            (
                "session_token".to_string(),
                AstarteData::String("abcd".to_string()),
            ),
            ("secure".to_string(), AstarteData::Boolean(false)),
        ]);

        let astarte_event = DeviceEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
        };

        let session = SessionInfo::from_event(astarte_event).unwrap();

        // the test is successful once handle_sessions terminates
        f.handle_sessions(session);
    }
}
