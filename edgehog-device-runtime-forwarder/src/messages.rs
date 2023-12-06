// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Internal Rust representation of protobuf structures.
//!
//! The structures belonging to this module are used to serialize/deserialize to/from the protobuf
//! data representation.

use std::borrow::{Borrow, BorrowMut};
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::num::TryFromIntError;
use std::ops::{Deref, DerefMut, Not};
use std::str::FromStr;

use displaydoc::Display;
use reqwest::{Client as ReqwClient, RequestBuilder};
use thiserror::Error as ThisError;
use url::ParseError;

use edgehog_device_forwarder_proto as proto;
use edgehog_device_forwarder_proto::{
    http::Message as ProtoHttpMessage,
    http::Request as ProtoHttpRequest,
    http::Response as ProtoHttpResponse,
    message::Protocol as ProtoProtocol,
    prost::{self, Message as ProstMessage},
    web_socket::Message as ProtoWsMessage,
    Http as ProtoHttp, WebSocket as ProtoWebSocket,
};

/// Errors occurring while handling [`protobuf`](https://protobuf.dev/overview/) messages
#[derive(Display, ThisError, Debug)]
#[non_exhaustive]
pub enum ProtocolError {
    /// Failed to serialize into Protobuf.
    Encode(#[from] prost::EncodeError),
    /// Failed to deserialize from Protobuf.
    Decode(#[from] prost::DecodeError),
    /// Empty fields.
    Empty,
    /// Reqwest error.
    Reqwest(#[from] reqwest::Error),
    /// Error parsing URL.
    ParseUrl(#[from] ParseError),
    /// Wrong HTTP method field.
    InvalidHttpMethod(#[from] http::method::InvalidMethod),
    /// Http error.
    Http(#[from] http::Error),
    /// Invalid HTTP status code
    InvalidStatusCode(#[from] http::status::InvalidStatusCode),
    /// Error while parsing Headers.
    ParseHeaders(#[from] http::header::ToStrError),
    /// Invalid port number
    InvalidPortNumber(#[from] TryFromIntError),
}

/// Requests Id.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct Id(Vec<u8>);

impl Debug for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Id({})", hex::encode(&self.0))
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}

impl Deref for Id {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Id {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Borrow<Vec<u8>> for Id {
    fn borrow(&self) -> &Vec<u8> {
        &self.0
    }
}

impl BorrowMut<Vec<u8>> for Id {
    fn borrow_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

impl From<Vec<u8>> for Id {
    fn from(value: Vec<u8>) -> Self {
        Id::new(value)
    }
}

impl Id {
    /// New Id.
    pub(crate) fn new(id: Vec<u8>) -> Self {
        Self(id)
    }
}

/// [`protobuf`](https://protobuf.dev/overview/) message internal representation.
#[derive(Debug)]
pub struct ProtoMessage {
    pub(crate) protocol: Protocol,
}

impl ProtoMessage {
    /// Create a [`ProtoMessage`].
    pub(crate) fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }

    /// Encode [`ProtoMessage`] struct into the corresponding [`protobuf`](https://protobuf.dev/overview/) version.
    pub(crate) fn encode(self) -> Result<Vec<u8>, ProtocolError> {
        let protocol = ProtoProtocol::from(self.protocol);

        let msg = proto::Message {
            protocol: Some(protocol),
        };

        let mut buf = Vec::with_capacity(msg.encoded_len());
        msg.encode(&mut buf)?;

        Ok(buf)
    }

    /// Decode a [`protobuf`](https://protobuf.dev/overview/) message into a [`ProtoMessage`] struct.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let msg = proto::Message::decode(bytes).map_err(ProtocolError::from)?;
        ProtoMessage::try_from(msg)
    }
}

impl TryFrom<proto::Message> for ProtoMessage {
    type Error = ProtocolError;

    fn try_from(value: proto::Message) -> Result<Self, Self::Error> {
        let proto::Message { protocol } = value;

        let protocol = protocol.ok_or(ProtocolError::Empty)?;

        Ok(ProtoMessage::new(protocol.try_into()?))
    }
}

/// Supported protocols.
#[derive(Debug)]
pub(crate) enum Protocol {
    Http(Http),
    WebSocket(WebSocket),
}

impl TryFrom<ProtoProtocol> for Protocol {
    type Error = ProtocolError;

    fn try_from(value: ProtoProtocol) -> Result<Self, Self::Error> {
        let protocol = match value {
            ProtoProtocol::Http(http) => Protocol::Http(http.try_into()?),
            ProtoProtocol::Ws(ws) => Protocol::WebSocket(ws.try_into()?),
        };

        Ok(protocol)
    }
}

impl From<Protocol> for ProtoProtocol {
    fn from(protocol: Protocol) -> Self {
        match protocol {
            Protocol::Http(http) => {
                let proto_http = ProtoHttp::from(http);
                ProtoProtocol::Http(proto_http)
            }
            Protocol::WebSocket(ws) => {
                let proto_ws = ProtoWebSocket::from(ws);

                ProtoProtocol::Ws(proto_ws)
            }
        }
    }
}

/// Http message.
#[derive(Debug)]
pub(crate) struct Http {
    /// Unique ID.
    pub(crate) request_id: Id,
    /// Http message type.
    pub(crate) http_msg: HttpMessage,
}

impl Http {
    pub(crate) fn new(request_id: Id, http_msg: HttpMessage) -> Self {
        Self {
            request_id,
            http_msg,
        }
    }
}

impl TryFrom<ProtoHttp> for Http {
    type Error = ProtocolError;

    fn try_from(value: ProtoHttp) -> Result<Self, Self::Error> {
        let ProtoHttp {
            request_id,
            message,
        } = value;

        if request_id.is_empty() || message.is_none() {
            return Err(ProtocolError::Empty);
        }

        let http_msg = match message.unwrap() {
            ProtoHttpMessage::Request(req) => {
                Ok::<HttpMessage, ProtocolError>(HttpMessage::Request(req.try_into()?))
            }
            ProtoHttpMessage::Response(res) => Ok(HttpMessage::Response(res.try_into()?)),
        }?;

        Ok(Http {
            request_id: request_id.into(),
            http_msg,
        })
    }
}

impl From<Http> for ProtoHttp {
    fn from(value: Http) -> Self {
        let message = match value.http_msg {
            HttpMessage::Request(req) => {
                let proto_req = ProtoHttpRequest::from(req);
                ProtoHttpMessage::Request(proto_req)
            }
            HttpMessage::Response(res) => {
                let proto_res = ProtoHttpResponse::from(res);
                ProtoHttpMessage::Response(proto_res)
            }
        };

        Self {
            request_id: value.request_id.0,
            message: Some(message),
        }
    }
}

/// Http protocol message types.
#[derive(Debug)]
pub(crate) enum HttpMessage {
    Request(HttpRequest),
    Response(HttpResponse),
}

/// HTTP request fields.
#[derive(Debug)]
pub(crate) struct HttpRequest {
    path: String,
    query_string: String,
    method: http::Method,
    headers: http::HeaderMap,
    body: Vec<u8>,
    /// Port on the device to which the request will be sent.
    port: u16,
}

impl HttpRequest {
    /// Create a [`RequestBuilder`] from an HTTP request message.
    pub(crate) fn request_builder(self) -> Result<RequestBuilder, ProtocolError> {
        let url_str = format!(
            "http://localhost:{}/{}?{}",
            self.port, self.path, self.query_string
        );
        let url = url::Url::parse(&url_str)?;
        let method = http::method::Method::from_str(self.method.as_str())?;

        let http_builder = ReqwClient::new()
            .request(method, url)
            .headers(self.headers)
            .body(self.body);

        Ok(http_builder)
    }
}

impl TryFrom<ProtoHttpRequest> for HttpRequest {
    type Error = ProtocolError;
    fn try_from(value: ProtoHttpRequest) -> Result<Self, Self::Error> {
        let ProtoHttpRequest {
            path,
            method,
            query_string,
            headers,
            body,
            port,
        } = value;
        Ok(Self {
            path,
            method: method.as_str().try_into()?,
            query_string,
            headers: (&headers).try_into()?,
            body,
            port: port.try_into()?,
        })
    }
}

impl From<HttpRequest> for ProtoHttpRequest {
    fn from(http_req: HttpRequest) -> Self {
        Self {
            path: http_req.path,
            method: http_req.method.as_str().to_string(),
            query_string: http_req.query_string,
            headers: headermap_to_hashmap(&http_req.headers),
            body: http_req.body,
            port: http_req.port.into(),
        }
    }
}

/// HTTP response fields.
#[derive(Debug)]
pub(crate) struct HttpResponse {
    status_code: http::StatusCode,
    headers: http::HeaderMap,
    body: Vec<u8>,
}

impl HttpResponse {
    /// Create a new HTTP response.
    pub(crate) fn new(
        status_code: http::StatusCode,
        headers: http::HeaderMap,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status_code,
            headers,
            body,
        }
    }

    /// Return the status code of the HTTP response.
    pub(crate) fn status(&self) -> u16 {
        self.status_code.as_u16()
    }
}

impl TryFrom<ProtoHttpResponse> for HttpResponse {
    type Error = ProtocolError;
    fn try_from(value: ProtoHttpResponse) -> Result<Self, Self::Error> {
        let ProtoHttpResponse {
            status_code,
            headers,
            body,
        } = value;

        Ok(Self {
            status_code: http::StatusCode::from_u16(status_code.try_into()?)?,
            headers: (&headers).try_into()?,
            body,
        })
    }
}

impl From<HttpResponse> for ProtoHttpResponse {
    fn from(http_res: HttpResponse) -> Self {
        Self {
            status_code: http_res.status_code.as_u16().into(),
            headers: headermap_to_hashmap(&http_res.headers),
            body: http_res.body,
        }
    }
}

/// WebSocket request fields.
#[derive(Debug)]
pub(crate) struct WebSocket {
    socket_id: Id,
    message: WebSocketMessage,
}

/// [`WebSocket`] message type.
#[derive(Debug)]
pub(crate) enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close { code: u16, reason: Option<String> },
}

impl TryFrom<ProtoWebSocket> for WebSocket {
    type Error = ProtocolError;

    fn try_from(value: ProtoWebSocket) -> Result<Self, Self::Error> {
        let proto::WebSocket { socket_id, message } = value;

        let Some(msg) = message else {
            return Err(Self::Error::Empty);
        };

        let message = match msg {
            ProtoWsMessage::Text(data) => WebSocketMessage::text(data),
            ProtoWsMessage::Binary(data) => WebSocketMessage::binary(data),
            ProtoWsMessage::Ping(data) => WebSocketMessage::ping(data),
            ProtoWsMessage::Pong(data) => WebSocketMessage::pong(data),
            ProtoWsMessage::Close(close) => WebSocketMessage::close(
                close.code.try_into()?,
                close.reason.is_empty().not().then_some(close.reason),
            ),
        };

        Ok(Self {
            socket_id: Id::new(socket_id),
            message,
        })
    }
}

impl From<WebSocket> for ProtoWebSocket {
    fn from(ws: WebSocket) -> Self {
        let ws_message = match ws.message {
            WebSocketMessage::Text(data) => ProtoWsMessage::Text(data),
            WebSocketMessage::Binary(data) => ProtoWsMessage::Binary(data),
            WebSocketMessage::Ping(data) => ProtoWsMessage::Ping(data),
            WebSocketMessage::Pong(data) => ProtoWsMessage::Pong(data),
            WebSocketMessage::Close { code, reason } => {
                ProtoWsMessage::Close(proto::web_socket::Close {
                    code: code.into(),
                    reason: reason.unwrap_or_default(),
                })
            }
        };

        proto::WebSocket {
            socket_id: ws.socket_id.0,
            message: Some(ws_message),
        }
    }
}

impl WebSocketMessage {
    /// Create a text frame.
    pub(crate) fn text(data: String) -> Self {
        Self::Text(data)
    }

    /// Create a binary frame.
    pub(crate) fn binary(data: Vec<u8>) -> Self {
        Self::Binary(data)
    }

    /// Create a ping frame.
    pub(crate) fn ping(data: Vec<u8>) -> Self {
        Self::Ping(data)
    }

    /// Create a pong frame.
    pub(crate) fn pong(data: Vec<u8>) -> Self {
        Self::Pong(data)
    }

    /// Create a close frame.
    pub(crate) fn close(code: u16, reason: Option<String>) -> Self {
        Self::Close { code, reason }
    }
}

/// Convert a [`HeaderMap`] containing all HTTP headers into a [`HashMap`].
pub(crate) fn headermap_to_hashmap<'a, I>(headers: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (&'a http::HeaderName, &'a http::HeaderValue)>,
{
    headers
        .into_iter()
        .map(|(name, val)| {
            (
                name.to_string(),
                String::from_utf8_lossy(val.as_bytes()).into(),
            )
        })
        .collect()
}
