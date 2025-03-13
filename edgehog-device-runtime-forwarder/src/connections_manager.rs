// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Handle the interaction between the device connections and Edgehog.

use std::ops::ControlFlow;

use backoff::{Error as BackoffError, ExponentialBackoff};
use futures::{future, SinkExt, StreamExt, TryFutureExt};
use thiserror::Error as ThisError;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver};
use tokio_tungstenite::{
    connect_async_tls_with_config, tungstenite::Error as TungError,
    tungstenite::Message as TungMessage, Connector, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, error, info, instrument, trace, warn};
use url::Url;

use crate::collection::Connections;
use crate::connection::ConnectionError;
use crate::messages::{Id, ProtoMessage, ProtocolError};
use crate::tls::{device_tls_config, Error as TlsError};

/// Size of the channels where to send proto messages.
pub(crate) const CHANNEL_SIZE: usize = 50;

/// Errors occurring during the connections management.
#[derive(displaydoc::Display, ThisError, Debug)]
#[non_exhaustive]
pub enum Error {
    /// Error performing exponential backoff when trying to (re)connect with Edgehog.
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
    /// Session token not present on URL
    TokenNotFound,
    /// Session token already in use
    TokenAlreadyUsed(String),
    /// Error while performing exponential backoff to create a WebSocket connection
    BackOff(#[from] BackoffError<Box<Error>>),
    /// Tls error
    Tls(#[from] TlsError),
}

/// WebSocket error causing disconnection.
#[derive(displaydoc::Display, ThisError, Debug)]
pub struct Disconnected(#[from] pub TungError);

/// WebSocket stream alias.
pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Handler responsible for
/// - establishing a WebSocket connection between a device and Edgehog
/// - receiving and sending data from/to it.
#[derive(Debug)]
pub struct ConnectionsManager {
    /// Collection of connections.
    pub(crate) connections: Connections,
    /// Websocket stream between the device and Edgehog.
    pub(crate) ws_stream: WsStream,
    /// Channel used to send through the WebSocket messages coming from each connection.
    pub(crate) rx_ws: Receiver<ProtoMessage>,
    /// Edgehog URL.
    pub(crate) url: Url,
    /// Flag to indicate if TLS should be enabled.
    pub(crate) secure: bool,
}

impl ConnectionsManager {
    /// Establish a new WebSocket connection between the device and Edgehog.
    #[instrument]
    pub async fn connect(url: Url, secure: bool) -> Result<Self, Error> {
        // compute the TLS connector information or use a plain ws connection
        let connector = if secure {
            device_tls_config()?
        } else {
            Connector::Plain
        };

        let ws_stream = Self::ws_connect(&url, connector).await?;

        // this channel is used by tasks associated with the current bridge-device session to exchange
        // available information on a given connection between the device and another service.
        // For instance, a device may have started a connection with a ttyd, a service used
        // for sharing a remote terminal over a WebSocket interface.
        let (tx_ws, rx_ws) = channel(CHANNEL_SIZE);

        let connections = Connections::new(tx_ws);

        Ok(Self {
            connections,
            ws_stream,
            rx_ws,
            url,
            secure,
        })
    }

    /// Perform exponential backoff while trying to connect with Edgehog.
    #[instrument(skip_all)]
    pub(crate) async fn ws_connect(
        url: &Url,
        connector: Connector,
    ) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, Error> {
        // try opening a WebSocket connection using exponential backoff
        let (ws_stream, http_res) =
            backoff::future::retry(ExponentialBackoff::default(), || async {
                debug!("creating WebSocket connection with {}", url);

                let connector_cpy = connector.clone();

                // if the connector id Connector::Plain, a plain ws connection will be established
                connect_async_tls_with_config(url, None, false, Some(connector_cpy))
                    .await
                    .map_err(|err| match err {
                        TungError::Http(http_res) if http_res.status().is_client_error() => {
                            error!(
                                "received HTTP client error ({}), stopping backoff",
                                http_res.status()
                            );

                            match get_token(url) {
                                Ok(token) => {
                                    BackoffError::Permanent(Error::TokenAlreadyUsed(token))
                                }
                                Err(err) => BackoffError::Permanent(err),
                            }
                        }
                        err => {
                            debug!("try reconnecting with backoff after tungstenite error: {err}");
                            BackoffError::Transient {
                                err: Error::WebSocket(err),
                                retry_after: None,
                            }
                        }
                    })
            })
            .await?;

        trace!("WebSocket response {http_res:?}");

        Ok(ws_stream)
    }

    /// Manage the reception and transmission of data between the WebSocket and each device connection.
    #[instrument(skip_all)]
    pub async fn handle_connections(&mut self) -> Result<(), Disconnected> {
        loop {
            match self.event_loop().await {
                Ok(ControlFlow::Continue(())) => {}
                // if a close frame has been received or the closing handshake is correctly
                // terminated, the manager terminates the handling of the connections
                Ok(ControlFlow::Break(())) | Err(TungError::ConnectionClosed) => break,
                // if the device received a message bigger than the maximum size, notify the error
                Err(TungError::Capacity(err)) => {
                    error!("capacity exceeded: {err}");
                    break;
                }
                Err(TungError::AlreadyClosed) => {
                    error!("BUG: trying to read/write on an already closed WebSocket");
                    break;
                }
                // if the connection has been suddenly interrupted, try re-establishing it.
                // only Tungstenite errors should be handled for device reconnection
                Err(err) => {
                    return Err(Disconnected(err));
                }
            }
        }

        Ok(())
    }

    /// Handle a single connection event.
    ///
    /// It performs specific operations depending on the occurrence of one of the following events:
    /// * Receiving data from the Edgehog-device WebSocket connection,
    /// * Receiving data from one of the device connections (e.g., between the device and TTYD).
    #[instrument(skip_all)]
    pub(crate) async fn event_loop(&mut self) -> Result<ControlFlow<()>, TungError> {
        let event = self.select_ws_event().await;

        match event {
            // receive data from Edgehog
            WebSocketEvents::Receive(msg) => {
                future::ready(msg)
                    .and_then(|msg| self.handle_tung_msg(msg))
                    .await
            }
            // receive data from a device connection (e.g., TTYD)
            WebSocketEvents::Send(tung_msg) => {
                let msg = match tung_msg.encode() {
                    Ok(msg) => TungMessage::Binary(msg.into()),
                    Err(err) => {
                        error!("discard message due to {err:?}");
                        return Ok(ControlFlow::Continue(()));
                    }
                };

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
            res = self.ws_stream.next() => {
                match res {
                    Some(msg) => {
                        trace!("received tungstenite message from Edgehog: {msg:?}");
                        WebSocketEvents::Receive(msg)
                    }
                    None => {
                        trace!("ws_stream next() returned None, connection already closed");
                        WebSocketEvents::Receive(Err(TungError::AlreadyClosed))
                    }
                }
            }
            next = self.rx_ws.recv() => match next {
                Some(msg) => {
                    trace!("proto message received from a device connection: {msg:?}");
                    WebSocketEvents::Send(Box::new(msg))
                }
                None => unreachable!("BUG: tx_ws channel should never be closed"),
            }
        }
    }

    /// Send a [`Tungstenite message`](tokio_tungstenite::tungstenite::Message) through the WebSocket toward Edgehog.
    #[instrument(skip_all)]
    pub(crate) async fn send_to_ws(&mut self, tung_msg: TungMessage) -> Result<(), TungError> {
        self.ws_stream.send(tung_msg).await
    }

    /// Handle a single WebSocket [`Tungstenite message`](tokio_tungstenite::tungstenite::Message).
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
            TungMessage::Pong(_) => debug!("received pong"),
            TungMessage::Close(close_frame) => {
                debug!("received close frame {close_frame:?}, closing active connections");
                self.disconnect();
                info!("closed every connection");
                return Ok(ControlFlow::Break(()));
            }
            // text frames should never be sent
            TungMessage::Text(data) => warn!("received Text WebSocket frame, {data}"),
            TungMessage::Binary(bytes) => {
                match ProtoMessage::decode(&bytes) {
                    // handle the actual protocol message
                    Ok(proto_msg) => {
                        trace!("message received from Edgehog: {proto_msg:?}");
                        if let Err(err) = self.handle_proto_msg(proto_msg).await {
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

    /// Handle a [`protocol message`](ProtoMessage).
    pub(crate) async fn handle_proto_msg(&mut self, proto_msg: ProtoMessage) -> Result<(), Error> {
        // remove from the collection all the terminated connections
        self.connections.remove_terminated();

        match proto_msg {
            ProtoMessage::Http(http) => {
                trace!("received HTTP message: {http:?}");
                self.connections.handle_http(http)
            }
            ProtoMessage::WebSocket(ws) => {
                trace!("received WebSocket frame: {ws:?}");
                self.connections.handle_ws(ws).await
            }
        }
    }

    /// Try to establish again a WebSocket connection with Edgehog in case the connection is lost.
    #[instrument(skip_all)]
    pub async fn reconnect(&mut self) -> Result<(), Error> {
        debug!("trying to reconnect");

        let connector = if self.secure {
            device_tls_config()?
        } else {
            Connector::Plain
        };

        self.ws_stream = Self::ws_connect(&self.url, connector).await?;

        info!("reconnected");
        Ok(())
    }

    /// Close all the connections the device has established (e.g., with TTYD).
    #[instrument(skip_all)]
    pub(crate) fn disconnect(&mut self) {
        self.connections.disconnect();
    }
}

/// Retrieve the session token query parameter from an URL
pub(crate) fn get_token(url: &Url) -> Result<String, Error> {
    url.query()
        .map(|s| s.trim_start_matches("session=").to_string())
        .ok_or(Error::TokenNotFound)
}

/// Possible events happening on a WebSocket connection.
pub(crate) enum WebSocketEvents {
    Receive(Result<TungMessage, TungError>),
    Send(Box<ProtoMessage>),
}
