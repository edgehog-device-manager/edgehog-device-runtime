// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Manage a single connection.
//!
//! A connection is responsible for sending and receiving data through a WebSocket connection from
//! and to the [`ConnectionsManager`](crate::connections_manager::ConnectionsManager).

pub mod http;
pub mod websocket;

use std::ops::Deref;

use async_trait::async_trait;
use thiserror::Error as ThisError;
use tokio::sync::mpsc::Sender;
use tokio::task::{JoinError, JoinHandle};
use tokio_tungstenite::tungstenite::Error as TungError;
use tracing::{error, instrument, trace};

use crate::messages::{Id, ProtoMessage, ProtocolError, WebSocketMessage as ProtoWebSocketMessage};

/// Size of the channel used to send messages from the [Connections Manager](crate::connections_manager::ConnectionsManager)
/// to a device WebSocket connection
pub(crate) const WS_CHANNEL_SIZE: usize = 50;

/// Connection errors.
#[non_exhaustive]
#[derive(displaydoc::Display, ThisError, Debug)]
pub enum ConnectionError {
    /// Channel error.
    Channel(&'static str),
    /// Reqwest error.
    Http(#[from] reqwest::Error),
    /// Protobuf error.
    Protobuf(#[from] ProtocolError),
    /// Failed to Join a task handle.
    JoinError(#[from] JoinError),
    /// Message sent to the wrong protocol
    WrongProtocol,
    /// Error when receiving message on WebSocket connection, `{0}`.
    WebSocket(#[from] Box<TungError>),
    /// Trying to poll while still connecting.
    Connecting,
}

/// Enum storing the write side of the channel used by the
/// [Connections Manager](crate::connections_manager::ConnectionsManager) to send WebSocket
/// messages to the respective connection that will handle it.
#[derive(Debug)]
pub(crate) enum WriteHandle {
    Http,
    Ws(Sender<ProtoWebSocketMessage>),
}

/// Handle to the task spawned to handle a [`Connection`].
#[derive(Debug)]
pub(crate) struct ConnectionHandle {
    /// Handle of the task managing the connection.
    pub(crate) handle: JoinHandle<()>,
    /// Handle necessary to send messages to the tokio task managing the connection.
    pub(crate) connection: WriteHandle,
}

impl Deref for ConnectionHandle {
    type Target = JoinHandle<()>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl ConnectionHandle {
    /// Once the connections manager receives a WebSocket message, it sends a message to the
    /// respective tokio task handling that connection.
    #[instrument]
    pub(crate) async fn send(&self, msg: ProtoMessage) -> Result<(), ConnectionError> {
        match &self.connection {
            WriteHandle::Http => Err(ConnectionError::Channel(
                "sending messages over a channel is only allowed for WebSocket connections",
            )),
            WriteHandle::Ws(tx_con) => {
                let message = msg.into_ws().ok_or(ConnectionError::WrongProtocol)?.message;
                tx_con.send(message).await.map_err(|_| {
                    ConnectionError::Channel(
                        "error while sending messages to the ConnectionsManager",
                    )
                })
            }
        }
    }
}

/// For each Connection implementing a given transport protocol (e.g., [`Http`], [`WebSocket`]), it
/// provides a method returning a [`protocol message`](ProtoMessage) to send to the
/// [`ConnectionsManager`](crate::collection::ConnectionsManager).
#[async_trait]
pub(crate) trait Transport {
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError>;
}

/// Trait used by each transport builder (e.g., [`HttpBuilder`], [`WebSocketBuilder`]) to build the
/// respective transport protocol struct.
#[async_trait]
pub(crate) trait TransportBuilder {
    type Connection: Transport;

    async fn build(
        self,
        id: &Id,
        tx_ws: Sender<ProtoMessage>,
    ) -> Result<Self::Connection, ConnectionError>;
}

/// Struct containing the connection information necessary to communicate with the
/// [`ConnectionsManager`](crate::collection::ConnectionsManager).
#[derive(Debug)]
pub(crate) struct Connection<T> {
    id: Id,
    tx_ws: Sender<ProtoMessage>,
    state: T,
}

impl<T> Connection<T> {
    /// Initialize a new connection.
    pub(crate) fn new(id: Id, tx_ws: Sender<ProtoMessage>, state: T) -> Self {
        Self { id, tx_ws, state }
    }

    /// Spawn the task responsible for handling the connection.
    #[instrument(skip_all)]
    pub(crate) fn spawn(self, write_handle: WriteHandle) -> ConnectionHandle
    where
        T: TransportBuilder + Send + 'static,
        <T as TransportBuilder>::Connection: Send,
    {
        // spawn a task responsible for notifying when new data is available
        let handle = tokio::spawn(async move { self.spawn_inner().await });

        ConnectionHandle {
            handle,
            connection: write_handle,
        }
    }

    #[instrument(skip_all, fields(id = %self.id))]
    async fn spawn_inner(self)
    where
        T: TransportBuilder + Send + 'static,
        <T as TransportBuilder>::Connection: Send,
    {
        if let Err(err) = self.task().await {
            error!("connection task failed with error {err:?}");
        }
    }

    /// Build the [`Transport`] and send protocol messages to the
    /// [ConnectionsManager](crate::connections_manager::ConnectionsManager).
    #[instrument(skip_all)]
    pub(crate) async fn task(self) -> Result<(), ConnectionError>
    where
        T: TransportBuilder,
    {
        // create a connection (either HTTP or WebSocket) which implements the Transport trait
        let mut connection = self.state.build(&self.id, self.tx_ws.clone()).await?;
        trace!("connection {} created", self.id);

        while let Some(proto_msg) = connection.next(&self.id).await? {
            self.tx_ws.send(proto_msg).await.map_err(|_| {
                ConnectionError::Channel(
                    "error while sending generic message to the ConnectionsManager",
                )
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        http::Http, ConnectionError, ConnectionHandle, Id, ProtoMessage, ProtoWebSocketMessage,
        Transport, WriteHandle, WS_CHANNEL_SIZE,
    };

    use crate::messages::{
        Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpRequest as ProtoHttpRequest,
        WebSocket as ProtoWebSocket,
    };

    use http::header::CONTENT_TYPE;
    use http::HeaderValue;
    use httpmock::MockServer;
    use tokio::sync::mpsc::channel;
    use url::Url;

    async fn empty_task() {}

    fn create_http_req_proto(url: Url) -> ProtoHttpRequest {
        ProtoHttpRequest {
            method: http::Method::GET,
            path: url.path().trim_start_matches('/').to_string(),
            query_string: url.query().unwrap_or_default().to_string(),
            headers: http::HeaderMap::new(),
            body: Vec::new(),
            port: url.port().expect("nonexistent port"),
        }
    }

    fn create_http_req_msg_proto(url: &str) -> ProtoMessage {
        let url = Url::parse(url).expect("failed to pars Url");

        ProtoMessage::Http(ProtoHttp::new(
            Id::try_from(b"1234".to_vec()).unwrap(),
            ProtoHttpMessage::Request(create_http_req_proto(url)),
        ))
    }

    #[tokio::test]
    async fn test_con_handle_send() {
        let (tx, mut rx) = channel::<ProtoWebSocketMessage>(WS_CHANNEL_SIZE);

        let con_handle = ConnectionHandle {
            handle: tokio::spawn(empty_task()),
            connection: WriteHandle::Ws(tx),
        };

        let proto_msg = ProtoMessage::WebSocket(ProtoWebSocket {
            socket_id: Id::try_from(b"1234".to_vec()).unwrap(),
            message: ProtoWebSocketMessage::Binary(b"message".to_vec()),
        });

        let res = con_handle.send(proto_msg).await;

        assert!(res.is_ok());

        let res = rx.recv().await.expect("channel error");
        let expected_res = ProtoWebSocketMessage::Binary(b"message".to_vec());

        assert_eq!(res, expected_res);
    }

    #[tokio::test]
    async fn test_con_handle_send_error() {
        // send() cannot be used in case the write handle is Http
        let con_handle = ConnectionHandle {
            handle: tokio::spawn(empty_task()),
            connection: WriteHandle::Http,
        };

        let proto_msg = ProtoMessage::WebSocket(ProtoWebSocket {
            socket_id: Id::try_from(b"1234".to_vec()).unwrap(),
            message: ProtoWebSocketMessage::Binary(b"message".to_vec()),
        });

        let res = con_handle.send(proto_msg).await;

        assert!(matches!(res, Err(ConnectionError::Channel(_))));

        // an error is returned in case the proto message is not of WebSocket type
        let (tx, _rx) = channel::<ProtoWebSocketMessage>(WS_CHANNEL_SIZE);
        let con_handle = ConnectionHandle {
            handle: tokio::spawn(empty_task()),
            connection: WriteHandle::Ws(tx),
        };

        let proto_msg = create_http_req_msg_proto("https://host:8080/path?session=abcd");
        let res = con_handle.send(proto_msg).await;

        assert!(matches!(res, Err(ConnectionError::WrongProtocol)));
    }

    #[tokio::test]
    async fn next_http() {
        let mock_server = MockServer::start();

        let mock_http_req = mock_server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/path")
                .query_param("session", "abcd");
            then.status(200)
                .header("content-type", "text/html")
                .body("body");
        });

        let url = mock_server.url("/path?session=abcd");

        let url = Url::parse(&url).expect("failed to parse Url");
        let http_rep = create_http_req_proto(url);

        let mut http = Http::new(
            http_rep
                .request_builder()
                .expect("failed to retrieve request builder"),
        );

        let id = Id::try_from(b"1234".to_vec()).unwrap();

        let res = http.next(&id).await.unwrap().unwrap();

        // check that there has been an HTTP call with the specified information
        mock_http_req.assert();

        let proto_msg = res.into_http().unwrap();
        assert_eq!(proto_msg.request_id, id);

        let res = proto_msg.http_msg.into_res().unwrap();
        assert_eq!(res.status_code, 200);
        assert_eq!(res.body, b"body");
        assert_eq!(
            res.headers.get(CONTENT_TYPE).unwrap(),
            HeaderValue::from_static("text/html")
        );

        // calling a second time next() on an http should return Ok(None)
        let res = http.next(&id).await;
        assert!(res.unwrap().is_none());
    }
}
