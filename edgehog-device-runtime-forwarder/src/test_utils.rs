// Copyright 2023-2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Module containing utility functions and structures to perform integration test of the library.

use crate::connections_manager::{ConnectionsManager, Disconnected};

use edgehog_device_forwarder_proto as proto;
use edgehog_device_forwarder_proto::{
    http::Message as ProtobufHttpMessage, http::Request as ProtobufHttpRequest,
    message::Protocol as ProtobufProtocol, prost::Message, web_socket::Close as ProtobufClose,
    web_socket::Message as ProtobufWsMessage, Http as ProtobufHttp, WebSocket as ProtobufWebSocket,
};
use futures::{SinkExt, StreamExt};
use httpmock::prelude::*;
use httpmock::Mock;
use std::collections::HashMap;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as TungMessage;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, instrument, warn};
use url::{ParseError, Url};

/// Build a listener on a free port.
pub async fn bind_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind("localhost:0")
        .await
        .expect("failed to create a tcp listener");

    let port = listener
        .local_addr()
        .expect("failed to retrieve local addr")
        .port();

    (listener, port)
}

/// Start a [`ConnectionsManager`] instance.
pub async fn con_manager(url: String, secure: bool) -> Result<(), Disconnected> {
    let mut con_manager = ConnectionsManager::connect(url.as_str().try_into().unwrap(), secure)
        .await
        .expect("failed to connect connections manager");
    con_manager.handle_connections().await
}

fn proto_http_req(request_id: Vec<u8>, url: &Url, body: Vec<u8>) -> proto::Message {
    proto::Message {
        protocol: Some(ProtobufProtocol::Http(ProtobufHttp {
            request_id,
            message: Some(ProtobufHttpMessage::Request(ProtobufHttpRequest {
                path: url.path().trim_start_matches('/').to_string(),
                method: "GET".to_string(),
                query_string: url.query().unwrap_or_default().to_string(),
                headers: HashMap::new(),
                body,
                port: url.port().expect("nonexistent port").into(),
            })),
        })),
    }
}

/// Create an HTTP request and wrap it into a [`tungstenite`](tokio_tungstenite::tungstenite) message.
pub fn create_http_req(request_id: Vec<u8>, url: &str, body: Vec<u8>) -> TungMessage {
    let url = Url::parse(url).expect("failed to pars Url");

    let proto_msg = proto_http_req(request_id, &url, body);

    let mut buf = Vec::with_capacity(proto_msg.encoded_len());
    proto_msg.encode(&mut buf).unwrap();

    TungMessage::Binary(buf)
}

/// Create an HTTP upgrade request and wrap it into a [`tungstenite`](tokio_tungstenite::tungstenite) message.
pub fn create_http_upgrade_req(request_id: Vec<u8>, url: &str) -> Result<TungMessage, ParseError> {
    let url = Url::parse(url)?;
    let port = url.port().ok_or(ParseError::InvalidPort)?.into();

    let mut headers = HashMap::new();
    headers.insert("Host".to_string(), format!("localhost:{port}"));
    headers.insert("Connection".to_string(), "keep-alive, Upgrade".to_string());
    headers.insert("Upgrade".to_string(), "websocket".to_string());
    headers.insert("Sec-WebSocket-Version".to_string(), "13".to_string());
    headers.insert("Sec-WebSocket-Protocol".to_string(), "tty".to_string());
    headers.insert(
        "Sec-WebSocket-Extensions".to_string(),
        "permessage-deflate".to_string(),
    );
    headers.insert(
        "Sec-WebSocket-Key".to_string(),
        "KZFI7tLjyq4dy8TqCPDRzA==".to_string(),
    );

    let proto_msg = proto::Message {
        protocol: Some(ProtobufProtocol::Http(ProtobufHttp {
            request_id,
            message: Some(ProtobufHttpMessage::Request(ProtobufHttpRequest {
                path: url.path().trim_start_matches('/').to_string(),
                method: "GET".to_string(),
                query_string: url.query().unwrap_or_default().to_string(),
                headers,
                body: Vec::new(),
                port,
            })),
        })),
    };

    let mut buf = Vec::with_capacity(proto_msg.encoded_len());
    proto_msg.encode(&mut buf).expect("not enough capacity");

    Ok(TungMessage::Binary(buf))
}

/// Check if the protobuf message contains an HTTP response upgrade
pub fn is_ws_upgrade_response(http_msg: ProtobufHttpMessage) -> bool {
    match http_msg {
        ProtobufHttpMessage::Request(_) => false,
        ProtobufHttpMessage::Response(res) => {
            res.status_code == 101 && res.headers.get("upgrade").unwrap().contains("websocket")
        }
    }
}

/// Create a binary [`tungstenite`](tokio_tungstenite::tungstenite) message carrying a WebSocket frame.
pub fn create_ws_msg(socket_id: Vec<u8>, frame: TungMessage) -> TungMessage {
    let proto_msg = proto::Message {
        protocol: Some(ProtobufProtocol::Ws(ProtobufWebSocket {
            socket_id,
            message: Some(match frame {
                TungMessage::Text(data) => ProtobufWsMessage::Text(data),
                TungMessage::Binary(data) => ProtobufWsMessage::Binary(data),
                TungMessage::Ping(data) => ProtobufWsMessage::Ping(data),
                TungMessage::Pong(data) => ProtobufWsMessage::Pong(data),
                TungMessage::Close(c) => match c {
                    None => ProtobufWsMessage::Close(ProtobufClose {
                        code: 1000,
                        reason: String::new(),
                    }),
                    Some(c) => ProtobufWsMessage::Close(ProtobufClose {
                        code: u16::from(c.code) as u32,
                        reason: c.reason.into_owned(),
                    }),
                },
                TungMessage::Frame(_) => unreachable!("shouldn't be sent"),
            }),
        })),
    };

    let mut buf = Vec::with_capacity(proto_msg.encoded_len());
    proto_msg.encode(&mut buf).unwrap();

    TungMessage::Binary(buf)
}

/// Send a message on a WebSocket stream, wait for a message on the stream and return it.
pub async fn send_ws_and_wait_next(
    ws_stream: &mut WebSocketStream<TcpStream>,
    data: TungMessage,
) -> proto::Message {
    ws_stream.send(data).await.expect("failed to send over ws");

    // should receive an HTTP response with status code 101, stating that the connection upgrade
    // was successful
    let http_res = ws_stream
        .next()
        .await
        .expect("ws already closed")
        .expect("failed to receive from ws")
        .into_data();

    Message::decode(http_res.as_slice()).expect("failed to create protobuf message")
}

/// Utility struct to test a connection (HTTP or WebSocket) with the device
#[derive(Debug)]
pub struct TestConnections<M> {
    /// Server used to mock the connections
    pub mock_server: M,
    listener: TcpListener,
    connections_handle: JoinHandle<Result<(), Disconnected>>,
}

impl<M> TestConnections<M> {
    /// Create a WebSocket connection and mock the Edgehog instance the device will connect to.
    pub async fn mock_ws_server(&self) -> WebSocketStream<TcpStream> {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .expect("failed to accept connection");

        tokio_tungstenite::accept_async(stream)
            .await
            .expect("failed to open a ws with the device")
    }

    /// Check if the connections manager correctly ended its execution.
    pub async fn assert(self) {
        let res = self.connections_handle.await.expect("task join failed");
        assert!(res.is_ok(), "connection manager error {}", res.unwrap_err());
    }
}

impl TestConnections<MockServer> {
    /// Initialize the HTTP mock server.
    pub async fn init() -> Self {
        let mock_server = MockServer::start();

        let (listener, port) = bind_port().await;
        let url = format!("ws://localhost:{port}/remote-terminal?session=abcd");

        Self {
            mock_server,
            listener,
            connections_handle: tokio::spawn(con_manager(url, false)),
        }
    }

    /// Retrieve the mock endpoint
    pub fn endpoint(&self) -> Mock<'_> {
        // Create a mock on the server.
        self.mock_server.mock(|when, then| {
            when.method(GET)
                .path("/remote-terminal")
                .query_param("session", "abcd");
            then.status(200)
                .header("content-type", "text/html")
                .body("just do it");
        })
    }
}

impl TestConnections<MockWebSocket> {
    /// Initialize the WebSocket mock server.
    pub async fn init() -> Self {
        let mock_server = MockWebSocket::start().await;

        let (listener, port) = bind_port().await;
        let url = format!("ws://localhost:{port}/remote-terminal?session=abcd");

        Self {
            mock_server,
            listener,
            connections_handle: tokio::spawn(con_manager(url, false)),
        }
    }

    /// Mock the WebSocket stream between the device and an internal service (e.g., TTYD)
    #[instrument(skip_all)]
    pub async fn mock(&mut self, connecting_handle: JoinHandle<WebSocketStream<TcpStream>>) {
        let ws_stream = connecting_handle.await.unwrap();
        let _handle = MockWebSocket::mock(ws_stream);
        self.mock_server.0 = WsState::Connected;
    }
}

/// WebSocket mock server
#[derive(Debug)]
pub struct MockWebSocket(WsState);

#[derive(Debug)]
enum WsState {
    Pending {
        listener: Option<TcpListener>,
        port: u16,
    },
    Connected,
}

impl MockWebSocket {
    /// Initialize the mock server.
    pub async fn start() -> Self {
        let (listener, port) = bind_port().await;
        Self(WsState::Pending {
            listener: Some(listener),
            port,
        })
    }

    /// Retrieve the [`TcpListener`] from a mock server in a Pending state.
    pub fn device_listener(&mut self) -> Option<TcpListener> {
        match &mut self.0 {
            WsState::Pending { listener, .. } => listener.take(),
            WsState::Connected => None,
        }
    }

    /// Retrieve the port the mock server will listen to new WebSocket connections.
    pub fn port(&self) -> Option<u16> {
        match self.0 {
            WsState::Pending { port, .. } => Some(port),
            _ => None,
        }
    }

    /// Check if the mock server established a WebSocket connection.
    pub fn is_connected(&self) -> bool {
        matches!(self.0, WsState::Connected)
    }

    /// Accept a WebSocket connection from a device request.
    #[instrument(skip_all)]
    pub fn open_ws_device(listener: TcpListener) -> JoinHandle<WebSocketStream<TcpStream>> {
        tokio::spawn(async move {
            debug!("creating stream at {listener:?}");

            let (stream, _) = listener
                .accept()
                .await
                .expect("failed to accept connection");

            tokio_tungstenite::accept_async(stream)
                .await
                .expect("failed to open a ws with the device")
        })
    }

    /// Spawn the task responsible for mocking a service behavior once received a WebSocket message
    /// from the device.
    pub fn mock(ws_stream: WebSocketStream<TcpStream>) -> JoinHandle<()> {
        tokio::spawn(Self::handle_ws(ws_stream))
    }

    async fn handle_ws(mut ws_stream: WebSocketStream<TcpStream>) {
        // loop endlessly. Upon receiving a message, forward it back
        while let Some(msg) = ws_stream.next().await {
            let msg = msg.expect("failed to receive from ws");
            // check what kind of frame is received
            let msg_response = match msg {
                // if binary, forward it
                TungMessage::Binary(data) => TungMessage::Binary(data),
                TungMessage::Close(_) => break,
                // if ping is received, send pong
                TungMessage::Ping(data) => TungMessage::Pong(data),
                // skip the following messages
                TungMessage::Pong(_) => {
                    debug!("received Pong frame");
                    continue;
                }
                TungMessage::Text(txt) => {
                    warn!("received Text frame with {txt}");
                    continue;
                }
                TungMessage::Frame(_) => {
                    warn!("should never be sent");
                    continue;
                }
            };

            ws_stream
                .send(msg_response)
                .await
                .expect("failed to send over ws");
        }
    }
}
