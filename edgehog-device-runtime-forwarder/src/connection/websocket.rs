// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Define the necessary structs and traits to represent a WebSocket connection.

use std::ops::ControlFlow;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use http::Request;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::tungstenite::{
    error::ProtocolError as TungProtocolError, Error as TungError, Message as TungMessage,
};
use tracing::{debug, error, instrument, trace};

use super::{
    Connection, ConnectionError, ConnectionHandle, Transport, TransportBuilder, WriteHandle,
    WS_CHANNEL_SIZE,
};

use crate::connections_manager::WsStream;
use crate::messages::{
    Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse, Id, ProtoMessage, WebSocketMessage as ProtoWebSocketMessage,
};

/// Builder for a [`WebSocket`] connection.
#[derive(Debug)]
pub(crate) struct WebSocketBuilder {
    request: Request<()>,
    rx_con: Receiver<ProtoWebSocketMessage>,
}

impl WebSocketBuilder {
    /// Check the HTTP upgrade request and build the channel used to send WebSocket messages to device services (e.g., TTYD).
    pub(crate) fn with_handle(
        http_req: ProtoHttpRequest,
    ) -> Result<(Self, WriteHandle), ConnectionError> {
        let request = http_req.ws_upgrade()?;
        trace!("HTTP request upgraded");

        // this channel will be used to send data from the manager to the WebSocket connection
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

        // send a protocol message with the HTTP response to the connections manager
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
    /// Write to or Read from a WebSocket.
    ///
    /// Returns a result only when the device receives a message from a WebSocket connection.
    /// If a message needs to be forwarded to the device's WebSocket connection, a recursive
    /// function call will be invoked.
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError> {
        match self.select().await {
            // message from internal WebSocket connection (e.g., with TTYD) to the connections manager
            WsEither::Read(tung_res) => self.handle_ws_read(id.clone(), tung_res).await,
            // message from the connections manager to the internal WebSocket connection
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
            Some(Err(TungError::Protocol(TungProtocolError::ResetWithoutClosingHandshake))) => {
                error!("closing connection due to reset without closing handshake");
                Ok(Some(ProtoMessage::try_from_tung(
                    id,
                    TungMessage::Close(Some(CloseFrame {
                        code: CloseCode::Protocol,
                        reason: Utf8Bytes::from_static("reset without closing handshake"),
                    })),
                )?))
            }
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
