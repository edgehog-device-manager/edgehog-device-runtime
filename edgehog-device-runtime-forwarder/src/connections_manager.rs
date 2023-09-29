// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Handle the interaction between the device connections and the bridge.

use std::ops::ControlFlow;
use std::time::Duration;

use backoff::ExponentialBackoff;
use displaydoc::Display;
use futures::{future, SinkExt, StreamExt, TryFutureExt};
use thiserror::Error as ThisError;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async, tungstenite::Error as TungError, tungstenite::Message as TungMessage,
    MaybeTlsStream, WebSocketStream,
};

use tracing::{debug, error, info, instrument, trace, warn};
use url::Url;

use crate::connection::ConnectionError;
use crate::connections::Connections;
use crate::messages::{
    Http, HttpMessage, Id, ProtoMessage, Protocol as ProtoProtocol, ProtocolError,
};

/// Size of the channels where to send proto messages.
pub(crate) const CHANNEL_SIZE: usize = 50;

/// Errors occurring during the connections management.
#[derive(Display, ThisError, Debug)]
#[non_exhaustive]
pub enum Error {
    /// Error performing exponential backoff when trying to (re)connect with the bridge.
    WebSocket(#[from] TungError),
    /// Protobuf error.
    Protobuf(#[from] ProtocolError),
    /// Connection error.
    Connection(#[from] ConnectionError),
    /// Wrong message with id `{0}`
    WrongMessage(Id),
    /// The connection does not exists, id: `{0}`.
    ConnectionNotFound(Id),
    /// Connection ID already in use, id: `{0}`.
    IdAlreadyUsed(Id),
    /// Unsupported message type
    Unsupported,
}

/// WebSocket stream alias.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The time interval in seconds that the device will wait before sending a Ping frame if no data is received from the WebSocket.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);

/// Handler responsible for establishing a websocket connection between a device and the bridge
/// and for receiving and sending data from/to it.
#[derive(Debug)]
pub struct ConnectionsManager {
    /// Collection of connections, each identified by an ID.
    connections: Connections,
    /// Websocket stream between the device and the bridge.
    ws_stream: WsStream,
    /// Channel used to send through the websocket messages coming from each connection.
    rx_ws: Receiver<ProtoMessage>,
    /// bridge URL.
    url: Url,
}

impl ConnectionsManager {
    /// Establish a new WebSocket connection between the device and the bridge.
    #[instrument]
    pub async fn connect(url: Url) -> Result<Self, Error> {
        // TODO: check if, when a wrong URL is passed, it will endlessly try to connect

        let ws_stream = Self::ws_connect(&url).await?;

        // this channel is used by tasks associated to each connection to communicate new
        // information available on a given websocket between the device and TTYD.
        // it is also used to forward the incoming data from TTYD to the device.
        let (tx_ws, rx_ws) = channel(CHANNEL_SIZE);

        let connections = Connections::new(tx_ws);

        Ok(Self {
            connections,
            ws_stream,
            rx_ws,
            url,
        })
    }

    /// Perform exponential backoff while trying to (re)connect with the bridge.
    #[instrument(skip_all)]
    pub(crate) async fn ws_connect(
        url: &Url,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, Error> {
        // try openning a websocket connection with the bridge using exponential backoff
        let (ws_stream, http_res) =
            backoff::future::retry(ExponentialBackoff::default(), || async {
                debug!("creating websocket connection with {}", url);
                Ok(connect_async(url).await?)
            })
            .await
            .map_err(Error::WebSocket)?;

        trace!("bridge websocket response {http_res:?}");

        Ok(ws_stream)
    }

    /// Manage the reception and transmission of data between the websocket and each connection.
    ///
    /// It performs specific operations depending on the occurrence of one of the following events:
    /// * Receiving data from the WebSocket,
    /// * A timeout event occurring before any data is received from the WebSocket connection,
    /// * Receiving data from one of the connections (e.g., between the device and TTYD).
    #[instrument(skip_all)]
    pub async fn handle_connections(&mut self) -> Result<(), Error> {
        loop {
            match self.event_loop().await {
                Ok(ControlFlow::Continue(())) => {}
                Ok(ControlFlow::Break(())) => break,
                // if the connection has been suddenly interrupted, try re-establishing it.
                // only Tungstenite errors should be handled for device reconnection
                Err(err) => {
                    error!("WebSocket error {err:?}");
                    self.reconnect().await?;
                }
            }
        }

        Ok(())
    }

    /// Handle a single connection event.
    #[instrument(skip_all)]
    pub(crate) async fn event_loop(&mut self) -> Result<ControlFlow<()>, TungError> {
        let event = self.select_ws_event().await;

        match event {
            // receive data from the bridge
            WebSocketEvents::Receive(msg) => {
                future::ready(msg)
                    .and_then(|msg| self.handle_tung_msg(msg))
                    .await
            }
            // receive data from a connection (e.g., TTYD)
            WebSocketEvents::Send(tung_msg) => {
                let msg = match tung_msg.encode() {
                    Ok(msg) => TungMessage::Binary(msg),
                    Err(err) => {
                        error!("discard message due to {err:?}");
                        return Ok(ControlFlow::Continue(()));
                    }
                };

                self.send_to_ws(msg)
                    .await
                    .map(|_| ControlFlow::Continue(()))
            }
            // in case no data is received in PING_TIMEOUT seconds over the websocket, send a ping.
            // TODO: no check is done to verify that a Pong frame has been received
            WebSocketEvents::Ping => {
                let msg = TungMessage::Ping(Vec::new());
                debug!("sending ping message");

                self.send_to_ws(msg)
                    .await
                    .map(|_| ControlFlow::Continue(()))
            }
        }
    }

    /// Check when a WebSocket event occurs.
    #[instrument(skip_all)]
    pub(crate) async fn select_ws_event(&mut self) -> WebSocketEvents {
        select! {
            res = timeout(PING_TIMEOUT, self.ws_stream.next()) => {
                match res {
                    Ok(Some(msg)) => WebSocketEvents::Receive(msg),
                     Ok(None) => WebSocketEvents::Receive(Err(tungstenite::Error::AlreadyClosed)),
                    Err(_) => WebSocketEvents::Ping,
                }
            }
            next = self.rx_ws.recv() => match next {
                Some(tung_msg) => WebSocketEvents::Send(tung_msg),
                None => unreachable!("BUG: tx_ws channel should never be closed"),
            }
        }
    }

    /// Send a [`Tungstenite message`](tungstenite::Message) through the WebSocket toward the bridge.
    #[instrument(skip_all)]
    pub(crate) async fn send_to_ws(&mut self, tung_msg: TungMessage) -> Result<(), TungError> {
        self.ws_stream.send(tung_msg).await
    }

    /// Handle a single WebSocket [`Tungstenite message`](tungstenite::Message).
    #[instrument(skip_all)]
    pub(crate) async fn handle_tung_msg(
        &mut self,
        msg: TungMessage,
    ) -> Result<ControlFlow<()>, TungError> {
        match msg {
            TungMessage::Ping(data) => {
                debug!("received ping, sending pong");
                let msg = TungMessage::Pong(data);
                self.send_to_ws(msg).await?;
            }
            TungMessage::Pong(_) => debug!("received Pong frame"),
            TungMessage::Close(close_frame) => {
                debug!("websocket close frame {close_frame:?}");
                self.disconnect();
                info!("closed every connection");
                return Ok(ControlFlow::Break(()));
            }
            // text frames should never be sent
            TungMessage::Text(data) => warn!("received Text websocket frame, {data}"),
            TungMessage::Binary(bytes) => {
                match ProtoMessage::decode(&bytes) {
                    // handle the actual protocol message
                    Ok(proto_msg) => {
                        trace!("message received from bridge: {proto_msg:?}");
                        if let Err(err) = self.handle_proto_msg(proto_msg) {
                            error!("failed to handle protobuf message due to {err:?}");
                        }
                    }
                    Err(err) => {
                        error!("failed to decode protobuf message due to {err:?}");
                    }
                }
            }
            // wrong Message type
            TungMessage::Frame(_) => error!("unhandled message type: {msg:?}"),
        }

        Ok(ControlFlow::Continue(()))
    }

    /// Handle a [`protobuf message`](ProtoMessage).
    pub(crate) fn handle_proto_msg(&mut self, proto_msg: ProtoMessage) -> Result<(), Error> {
        // handle only HTTP requests, not other kind of protobuf messages
        match proto_msg.protocol {
            ProtoProtocol::Http(Http {
                request_id,
                http_msg: HttpMessage::Request(http_req),
            }) => self.connections.handle_http(request_id, http_req),
            ProtoProtocol::Http(Http {
                request_id,
                http_msg: HttpMessage::Response(_http_res),
            }) => {
                error!("Http response should not be sent by the bridge");
                Err(Error::WrongMessage(request_id))
            }
            ProtoProtocol::WebSocket(_ws) => {
                error!("WebSocket messages are not supported yet");
                Err(Error::Unsupported)
            }
        }
    }

    /// Try to establish again a WebSocket connection with the bridge in case the connection is lost.
    #[instrument(skip_all)]
    pub(crate) async fn reconnect(&mut self) -> Result<(), Error> {
        debug!("trying to reconnect");

        self.ws_stream = Self::ws_connect(&self.url).await?;

        info!("reconnected");

        Ok(())
    }

    /// Close all the connections the device has established (e.g., with TTYD).
    #[instrument(skip_all)]
    pub(crate) fn disconnect(&mut self) {
        info!("closing all the connections");
        self.connections.disconnect();
    }
}

/// Possible events happening on a WebSocket connection.
pub(crate) enum WebSocketEvents {
    Receive(Result<TungMessage, TungError>),
    Send(ProtoMessage),
    Ping,
}
