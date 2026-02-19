// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Define the necessary structs and traits to represent an HTTP connection.

use tokio::sync::mpsc::Sender;
use tracing::{debug, instrument, trace};

use super::{
    Connection, ConnectionError, ConnectionHandle, Transport, TransportBuilder, WriteHandle,
};
use crate::messages::{
    Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse, Id, ProtoMessage,
};

/// Builder for an [`Http`] connection.
#[derive(Debug)]
pub(crate) struct HttpBuilder {
    request: reqwest::RequestBuilder,
}

impl HttpBuilder {
    fn new(request: reqwest::RequestBuilder) -> Self {
        Self { request }
    }
}

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
    // to send the request the builder must be consumed, so the option can be replaced with None.
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

impl Transport for Http {
    /// Send the [HTTP request](reqwest::Request), wait for a response and return it.
    #[instrument(skip(self))]
    async fn next(&mut self, id: &Id) -> Result<Option<ProtoMessage>, ConnectionError> {
        let Some(request) = self.request.take() else {
            return Ok(None);
        };

        trace!("sending HTTP request");
        match request.send().await {
            Ok(http_res) => {
                // create the protobuf response to be sent to Edgehog
                let proto_res = ProtoHttpResponse::from_reqw_response(http_res).await?;

                let proto_msg = ProtoMessage::Http(ProtoHttp::new(
                    id.clone(),
                    ProtoHttpMessage::Response(proto_res),
                ));

                Ok(Some(proto_msg))
            }
            Err(err) => {
                debug!("HTTP request failed: {err}");
                let proto_msg = ProtoMessage::Http(ProtoHttp::bad_gateway(id.clone()));
                Ok(Some(proto_msg))
            }
        }
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
