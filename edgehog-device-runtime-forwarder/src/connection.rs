// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Manage a single connection.
//!
//! A connection is responsible for sending and receiving data through a WebSocket connection from
//! and to the [`ConnectionsManager`](crate::connections_manager::ConnectionsManager).

use std::ops::{ControlFlow, Deref, DerefMut};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use http::Request;
use thiserror::Error as ThisError;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::{JoinError, JoinHandle};
use tokio_tungstenite::tungstenite::{Error as TungError, Message as TungMessage};
use tracing::{debug, error, instrument, trace};

use crate::connections_manager::WsStream;
use crate::messages::{
    Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse, Id, ProtoMessage, ProtocolError,
    WebSocketMessage as ProtoWebSocketMessage,
};

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
    /// Error when receiving message on websocket connection, `{0}`.
    WebSocket(#[from] TungError),
    /// Trying to poll while still connecting.
    Connecting,
}

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
    /// Handle used to send messages to the tokio task of a certain connection.
    pub(crate) connection: WriteHandle,
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

impl Deref for ConnectionHandle {
    type Target = JoinHandle<()>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl DerefMut for ConnectionHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
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

/// For each Connection implementing a given transport protocol (e.g., [`Http`], [`WebSocket`]), it
/// provides a method returning a [`protocol message`](ProtoMessage) to send to the connections
/// manager.
#[async_trait]
pub(crate) trait Transport {
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError>;
}

/// Builder for an [`Http`] connection.
#[derive(Debug)]

pub(crate) struct HttpBuilder {
    /// To send the request the builder must be consumed, so the option can be replaced with None
    request: reqwest::RequestBuilder,
}

impl HttpBuilder {
    fn new(request: reqwest::RequestBuilder) -> Self {
        Self { request }
    }
}

#[async_trait]
impl TransportBuilder for HttpBuilder {
    type Connection = Http;

    /// The id and the channel are only useful for [`WebSocket`] connections.
    async fn build(
        self,
        _id: &Id,
        _tx_ws: Sender<ProtoMessage>,
    ) -> Result<Self::Connection, ConnectionError> {
        Ok(Http::new(self.request))
    }
}

impl TryFrom<ProtoHttpRequest> for HttpBuilder {
    type Error = ConnectionError;

    fn try_from(value: ProtoHttpRequest) -> Result<Self, Self::Error> {
        value
            .request_builder()
            .map(HttpBuilder::new)
            .map_err(ConnectionError::from)
    }
}

/// HTTP connection protocol
#[derive(Debug)]
pub(crate) struct Http {
    /// To send the request the builder must be consumed, so the option can be replaced with None.
    request: Option<reqwest::RequestBuilder>,
}

impl Http {
    /// Store the HTTP request the connection will respond to once executed.
    pub(crate) fn new(request: reqwest::RequestBuilder) -> Self {
        Self {
            request: Some(request),
        }
    }
}

#[async_trait]
impl Transport for Http {
    /// Send the [HTTP request](reqwest::Request), wait for a response and return it.
    #[instrument(skip(self))]
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError> {
        let Some(request) = self.request.take() else {
            return Ok(None);
        };

        trace!("sending HTTP request");
        let http_res = request.send().await?;

        // create the protobuf response to send to the bridge
        let proto_res = ProtoHttpResponse::from_reqw_response(http_res).await?;

        debug!("response code {}", proto_res.status());

        let proto_msg = ProtoMessage::Http(ProtoHttp::new(
            id.clone(),
            ProtoHttpMessage::Response(proto_res),
        ));

        Ok(Some(proto_msg))
    }
}

/// Builder for an [`WebSocket`] connection.
#[derive(Debug)]
pub(crate) struct WebSocketBuilder {
    request: Request<()>,
    rx_con: Receiver<ProtoWebSocketMessage>,
}

impl WebSocketBuilder {
    /// Upgrade the HTTP request and build the channel used to send WebSocket messages to device
    /// services (e.g., TTYD).
    fn with_handle(http_req: ProtoHttpRequest) -> Result<(Self, WriteHandle), ConnectionError> {
        let request = http_req.ws_upgrade()?;
        trace!("HTTP request upgraded");

        // this channel that will be used to send data from the manager to the websocket connection
        let (tx_con, rx_con) = channel::<ProtoWebSocketMessage>(WS_CHANNEL_SIZE);

        Ok((Self { request, rx_con }, WriteHandle::Ws(tx_con)))
    }
}

#[async_trait]
impl TransportBuilder for WebSocketBuilder {
    type Connection = WebSocket;

    #[instrument(skip(self, tx_ws))]
    async fn build(
        self,
        id: &Id,
        tx_ws: Sender<ProtoMessage>,
    ) -> Result<Self::Connection, ConnectionError> {
        // establish a WebSocket connection
        let (ws_stream, http_res) = tokio_tungstenite::connect_async(self.request).await?;
        trace!("WebSocket stream for ID {id} created");

        // send a ProtoMessage with the HTTP generated response to the connections manager
        let proto_msg = ProtoMessage::Http(ProtoHttp::new(
            id.clone(),
            ProtoHttpMessage::Response(ProtoHttpResponse::try_from(http_res)?),
        ));

        tx_ws.send(proto_msg).await.map_err(|_| {
            ConnectionError::Channel(
                "error while returning the Http upgrade response to the ConnectionsManager",
            )
        })?;

        Ok(WebSocket::new(ws_stream, self.rx_con))
    }
}

/// WebSocket connection protocol.
#[derive(Debug)]
pub(crate) struct WebSocket {
    ws_stream: WsStream,
    rx_con: Receiver<ProtoWebSocketMessage>,
}

#[async_trait]
impl Transport for WebSocket {
    /// Write/Read to/from a WebSocket.
    ///
    /// Returns a result only when the device receives a message from a WebSocket connection.
    /// If a message needs to be forwarded to the device's WebSocket connection, a recursive
    /// function call will be invoked.
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError> {
        match self.select().await {
            // message from internal websocket connection (e.g., with TTYD) to the connections manager
            WsEither::Read(tung_res) => self.handle_ws_read(id.clone(), tung_res).await,
            // message from the connections manager to the internal websocket connection
            WsEither::Write(chan_data) => {
                if let ControlFlow::Break(()) = self.handle_ws_write(chan_data).await? {
                    return Ok(None);
                }
                self.next(id).await
            }
        }
    }
}

impl WebSocket {
    fn new(ws_stream: WsStream, rx_con: Receiver<ProtoWebSocketMessage>) -> Self {
        Self { ws_stream, rx_con }
    }

    /// The device can either receive a message from the WebSocket connection or may need to
    /// forward data to it.
    async fn select(&mut self) -> WsEither {
        select! {
            tung_res = self.ws_stream.next() => WsEither::Read(tung_res),
            chan_data = self.rx_con.recv() => WsEither::Write(chan_data)
        }
    }

    /// Handle the reception of new data from a WebSocket connection.
    #[instrument(skip(self, tung_res))]
    async fn handle_ws_read(
        &mut self,
        id: Id,
        tung_res: Option<Result<TungMessage, TungError>>,
    ) -> Result<Option<ProtoMessage>, ConnectionError> {
        match tung_res {
            // ws stream closed
            None => {
                debug!("ws stream {id} has been closed, exit");
                Ok(None)
            }
            Some(Ok(tung_msg)) => Ok(Some(ProtoMessage::try_from_tung(id, tung_msg)?)),
            Some(Err(err)) => Err(err.into()),
        }
    }

    /// Forward data from the [`ConnectionsManager`](crate::connections_manager::ConnectionsManager)
    /// to the device WebSocket connection.
    #[instrument(skip_all)]
    async fn handle_ws_write(
        &mut self,
        chan_data: Option<ProtoWebSocketMessage>,
    ) -> Result<ControlFlow<()>, ConnectionError> {
        // convert the websocket proto message into a Tung message
        match chan_data {
            None => {
                debug!("channel dropped, closing connection");
                Ok(ControlFlow::Break(()))
            }
            Some(ws_msg) => {
                self.ws_stream.send(ws_msg.into()).await?;
                trace!("message sent to TTYD");
                Ok(ControlFlow::Continue(()))
            }
        }
    }
}

/// Utility enum to avoid having too much code in the [`select`] macro branches.
enum WsEither {
    Read(Option<Result<TungMessage, TungError>>),
    Write(Option<ProtoWebSocketMessage>),
}

/// Struct containing a connection information useful to communicate with the
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

    /// Send an HTTP request, wait for a response, build a protobuf message and send it to the
    /// [ConnectionsManager](crate::connections_manager::ConnectionsManager).
    #[instrument(skip_all, fields(id = %self.id))]
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

impl Connection<HttpBuilder> {
    /// Initialize a new Http connection.
    #[instrument(skip(tx_ws, http_req))]
    pub(crate) fn with_http(
        id: Id,
        tx_ws: Sender<ProtoMessage>,
        http_req: ProtoHttpRequest,
    ) -> Result<ConnectionHandle, ConnectionError> {
        let http_builder = HttpBuilder::try_from(http_req)?;
        let con = Self::new(id, tx_ws, http_builder);
        Ok(con.spawn(WriteHandle::Http))
    }
}

impl Connection<WebSocketBuilder> {
    /// Initialize a new WebSocket connection.
    #[instrument(skip(tx_ws, http_req))]
    pub(crate) fn with_ws(
        id: Id,
        tx_ws: Sender<ProtoMessage>,
        http_req: ProtoHttpRequest,
    ) -> Result<ConnectionHandle, ConnectionError> {
        let (ws_builder, write_handle) = WebSocketBuilder::with_handle(http_req)?;
        let con = Self::new(id, tx_ws, ws_builder);
        Ok(con.spawn(write_handle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::WebSocket as ProtoWebSocket;
    use http::header::CONTENT_TYPE;
    use http::HeaderValue;
    use httpmock::Method::GET;
    use httpmock::MockServer;
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
            message: ProtoWebSocketMessage::binary(b"message".to_vec()),
        });

        let res = con_handle.send(proto_msg).await;

        assert!(res.is_ok());

        let res = rx.recv().await.expect("channel error");
        let expected_res = ProtoWebSocketMessage::binary(b"message".to_vec());

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
            message: ProtoWebSocketMessage::binary(b"message".to_vec()),
        });

        let res = con_handle.send(proto_msg).await;

        assert!(matches!(res, Err(ConnectionError::Channel(_))));

        // an error is returned in case the proto message is not of websocket type
        let (tx, _rx) = channel::<ProtoWebSocketMessage>(WS_CHANNEL_SIZE);
        let con_handle = ConnectionHandle {
            handle: tokio::spawn(empty_task()),
            connection: WriteHandle::Ws(tx),
        };

        let proto_msg = create_http_req_msg_proto("https://host:8080/path?session_token=abcd");
        let res = con_handle.send(proto_msg).await;

        assert!(matches!(res, Err(ConnectionError::WrongProtocol)));
    }

    #[tokio::test]
    async fn next_http() {
        let mock_server = MockServer::start();

        let mock_http_req = mock_server.mock(|when, then| {
            when.method(GET)
                .path("/path")
                .query_param("session_token", "abcd");
            then.status(200)
                .header("content-type", "text/html")
                .body("body");
        });

        let url = mock_server.url("/path?session_token=abcd");

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
