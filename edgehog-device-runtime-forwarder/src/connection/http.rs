// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Define the necessary structs and traits to represent an HTTP connection.

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use tracing::{debug, instrument, trace};

use super::{ConnectionError, Transport, TransportBuilder};
use crate::messages::{
    Http as ProtoHttp, HttpMessage as ProtoHttpMessage, HttpRequest as ProtoHttpRequest,
    HttpResponse as ProtoHttpResponse, Id, ProtoMessage,
};

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
