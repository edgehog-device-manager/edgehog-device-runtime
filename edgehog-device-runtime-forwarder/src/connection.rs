// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Manage a single connection.
//!
//! A connection is responsible for sending and receiving data through a WebSocket connection from
//! and to the [`ConnectionsManager`](crate::connections_manager::ConnectionsManager).

use std::ops::{Deref, DerefMut};

use thiserror::Error as ThisError;
use tokio::sync::mpsc::Sender;
use tokio::task::{JoinError, JoinHandle};
use tracing::{debug, error, instrument, span, Level};

use crate::messages::{
    Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpResponse, Id, ProtoMessage,
    ProtocolError,
};

/// Connection errors.
#[non_exhaustive]
#[derive(displaydoc::Display, ThisError, Debug)]
pub enum ConnectionError {
    /// Channel error.
    ChannelToWs,
    /// Reqwest error.
    Reqwest(#[from] reqwest::Error),
    /// Protobuf error.
    Protobuf(#[from] ProtocolError),
    /// Failed to Join a task handle.
    JoinError(#[from] JoinError),
}

/// Handle to the task spawned to handle a [`Connection`].
#[derive(Debug)]
pub(crate) struct ConnectionHandle {
    /// Handle of the task managing the connection.
    pub(crate) handle: JoinHandle<()>,
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

/// Struct containing a connection information useful to communicate with the
/// [`ConnectionsManager`](crate::collection::ConnectionsManager).
#[derive(Debug)]
pub(crate) struct Connection {
    id: Id,
    tx_ws: Sender<ProtoMessage>,
    request: reqwest::RequestBuilder,
}

impl Connection {
    /// Initialize a new connection.
    pub(crate) fn new(
        id: Id,
        tx_ws: Sender<ProtoMessage>,
        request: reqwest::RequestBuilder,
    ) -> Self {
        Self { id, tx_ws, request }
    }

    /// Spawn the task responsible for handling the connection.
    #[instrument(skip_all)]
    pub(crate) fn spawn(self) -> ConnectionHandle {
        // spawn a task responsible for notifying when new data is available
        let handle = tokio::spawn(async move {
            // the span in used to know from which request a possible error is generated
            let span = span!(Level::DEBUG, "spawn", id = %self.id);
            let _enter = span.enter();

            if let Err(err) = self.task().await {
                error!("connection task failed with error {err:?}");
            }
        });

        ConnectionHandle { handle }
    }

    /// Send an HTTP request, wait for a response, build a protobuf message and send it to the
    /// [`ConnectionsManager`](crate::collection::ConnectionsManager).
    #[instrument(skip_all, fields(id = %self.id))]
    async fn task(self) -> Result<(), ConnectionError> {
        let http_res = self.request.send().await?;

        let status_code = http_res.status();
        let headers = http_res.headers().clone();
        let body = http_res.bytes().await?.into();

        let proto_res = HttpResponse::new(status_code, headers, body);

        debug!("response code {}", proto_res.status());

        let proto_msg = ProtoMessage::Http(ProtoHttp::new(
            self.id,
            ProtoHttpMessage::Response(proto_res),
        ));

        self.tx_ws
            .send(proto_msg)
            .await
            .map_err(|_| ConnectionError::ChannelToWs)
    }
}
