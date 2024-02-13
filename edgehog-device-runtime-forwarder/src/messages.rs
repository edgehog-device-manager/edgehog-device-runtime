// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Internal Rust representation of protobuf structures.
//!
//! The structures belonging to this module are used to serialize/deserialize to/from the protobuf
//! data representation.

use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter};
use std::num::TryFromIntError;
use std::ops::Not;
use std::str::FromStr;

use displaydoc::Display;
use thiserror::Error as ThisError;
use url::ParseError;

use edgehog_device_forwarder_proto as proto;
use edgehog_device_forwarder_proto::{
    http::Message as ProtoHttpMessage,
    http::Request as ProtoHttpRequest,
    http::Response as ProtoHttpResponse,
    message::Protocol as ProtoProtocol,
    prost::{self, Message as ProstMessage},
    web_socket::Close as ProtoWsClose,
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

impl TryFrom<Vec<u8>> for Id {
    type Error = ProtocolError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(ProtocolError::Empty);
        }

        Ok(Self(value))
    }
}

/// [`protobuf`](https://protobuf.dev/overview/) message internal representation.
///
/// It contains the actually supported protocols.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ProtoMessage {
    Http(Http),
    WebSocket(WebSocket),
}

impl ProtoMessage {
    /// Encode [`ProtoMessage`] struct into the corresponding [`protobuf`](https://protobuf.dev/overview/) version.
    pub(crate) fn encode(self) -> Result<Vec<u8>, ProtocolError> {
        let protocol = ProtoProtocol::from(self);

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
        Self::try_from(msg)
    }
}

impl TryFrom<proto::Message> for ProtoMessage {
    type Error = ProtocolError;

    fn try_from(value: proto::Message) -> Result<Self, Self::Error> {
        let proto::Message { protocol } = value;

        let protocol = protocol.ok_or(ProtocolError::Empty)?;

        protocol.try_into()
    }
}

impl TryFrom<ProtoProtocol> for ProtoMessage {
    type Error = ProtocolError;

    fn try_from(value: ProtoProtocol) -> Result<Self, Self::Error> {
        let protocol = match value {
            ProtoProtocol::Http(http) => ProtoMessage::Http(http.try_into()?),
            ProtoProtocol::Ws(ws) => ProtoMessage::WebSocket(ws.try_into()?),
        };

        Ok(protocol)
    }
}

impl From<ProtoMessage> for ProtoProtocol {
    fn from(protocol: ProtoMessage) -> Self {
        match protocol {
            ProtoMessage::Http(http) => {
                let proto_http = ProtoHttp::from(http);
                ProtoProtocol::Http(proto_http)
            }
            ProtoMessage::WebSocket(ws) => {
                let proto_ws = ProtoWebSocket::from(ws);

                ProtoProtocol::Ws(proto_ws)
            }
        }
    }
}

/// Http message.
#[derive(Debug, Eq, PartialEq)]
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

        let request_id = request_id.try_into()?;

        message
            .ok_or(ProtocolError::Empty)
            .and_then(|msg| match msg {
                ProtoHttpMessage::Request(req) => req.try_into().map(HttpMessage::Request),
                ProtoHttpMessage::Response(res) => res.try_into().map(HttpMessage::Response),
            })
            .map(|http_msg: HttpMessage| Http {
                request_id,
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
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum HttpMessage {
    Request(HttpRequest),
    Response(HttpResponse),
}

/// HTTP request fields.
#[derive(Debug, Eq, PartialEq)]
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
    /// Create a [`RequestBuilder`](reqwest::RequestBuilder) from an HTTP request message.
    pub(crate) fn request_builder(self) -> Result<reqwest::RequestBuilder, ProtocolError> {
        let url_str = format!(
            "http://localhost:{}/{}?{}",
            self.port, self.path, self.query_string
        );
        let url = url::Url::parse(&url_str)?;
        let method = http::method::Method::from_str(self.method.as_str())?;

        let http_builder = reqwest::Client::new()
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
#[derive(Debug, Eq, PartialEq)]
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
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct WebSocket {
    socket_id: Id,
    message: WebSocketMessage,
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
            socket_id: Id::try_from(socket_id)?,
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
            WebSocketMessage::Close { code, reason } => ProtoWsMessage::Close(ProtoWsClose {
                code: code.into(),
                reason: reason.unwrap_or_default(),
            }),
        };

        proto::WebSocket {
            socket_id: ws.socket_id.0,
            message: Some(ws_message),
        }
    }
}

/// [`WebSocket`] message type.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum WebSocketMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close { code: u16, reason: Option<String> },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn http_message_req() -> HttpMessage {
        HttpMessage::Request(HttpRequest {
            method: http::Method::GET,
            path: String::new(),
            query_string: String::new(),
            headers: http::HeaderMap::new(),
            body: Vec::new(),
            port: 0,
        })
    }

    fn empty_http(id: &[u8]) -> Http {
        Http {
            request_id: Id::try_from(id.to_vec()).unwrap(),
            http_msg: http_message_req(),
        }
    }

    fn empty_protobuf_http(id: &[u8]) -> ProtoHttp {
        ProtoHttp {
            request_id: id.to_vec(),
            message: Some(ProtoHttpMessage::Request(ProtoHttpRequest {
                body: Vec::new(),
                headers: HashMap::new(),
                query_string: String::new(),
                path: String::new(),
                method: "GET".to_string(),
                port: 0,
            })),
        }
    }

    fn empty_protobuf_ws(id: &[u8]) -> ProtoWebSocket {
        ProtoWebSocket {
            socket_id: id.to_vec(),
            message: Some(ProtoWsMessage::Binary(b"test_data".to_vec())),
        }
    }

    #[test]
    fn test_id() {
        // test empty ID
        assert!(matches!(
            Id::try_from(Vec::new()),
            Err(ProtocolError::Empty)
        ));

        let id_binary = b"test_id".to_vec();
        let id = Id::try_from(id_binary.clone()).unwrap();

        // test Display
        let display_id = format!("{id}");
        let res = hex::decode(display_id).unwrap();

        assert_eq!(res, id_binary);

        // test Debug
        let debug_id = format!("{id:?}");

        assert_eq!(debug_id, format!("Id({id})"));
    }

    #[test]
    fn test_from_protobuf_protocol() {
        // test WebSocket match case
        let id = b"test_id".to_vec();
        let proto = ProtoProtocol::Ws(empty_protobuf_ws(&id));
        let res = ProtoMessage::try_from(proto).unwrap();

        let exp = ProtoMessage::WebSocket(WebSocket {
            socket_id: Id::try_from(id).unwrap(),
            message: WebSocketMessage::Binary(b"test_data".to_vec()),
        });

        assert_eq!(res, exp);
    }

    #[test]
    fn test_try_from_protobuf_http() {
        // test response ok
        let protobuf_msg = ProtoHttp {
            request_id: b"test_id".to_vec(),
            message: Some(ProtoHttpMessage::Response(ProtoHttpResponse {
                body: Vec::new(),
                headers: HashMap::new(),
                status_code: 200,
            })),
        };

        assert!(Http::try_from(protobuf_msg).is_ok());

        // test missing message
        let protobuf_msg = ProtoHttp {
            request_id: b"test_id".to_vec(),
            message: None,
        };

        assert!(matches!(
            Http::try_from(protobuf_msg),
            Err(ProtocolError::Empty)
        ));
    }

    #[test]
    fn test_from_http() {
        let msg = empty_http(b"test_id");

        let expected = empty_protobuf_http(b"test_id");

        assert_eq!(ProtoHttp::from(msg), expected);
    }

    #[test]
    fn test_status() {
        let http_res = HttpResponse {
            status_code: http::StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: Vec::new(),
        };

        assert_eq!(200, http_res.status());
    }

    #[test]
    fn test_try_from_protobuf_websocket() {
        // empty ws message
        let protobuf_msg = ProtoWebSocket {
            socket_id: b"test_id".to_vec(),
            message: None,
        };

        assert!(matches!(
            WebSocket::try_from(protobuf_msg),
            Err(ProtocolError::Empty)
        ));

        // empty ID message
        let protobuf_msg = ProtoWebSocket {
            socket_id: Vec::new(),
            message: Some(ProtoWsMessage::Binary(Vec::new())),
        };

        assert!(matches!(
            WebSocket::try_from(protobuf_msg),
            Err(ProtocolError::Empty)
        ));

        // check all variants
        let protobuf_msgs = [
            (
                ProtoWsMessage::Text(String::new()),
                WebSocketMessage::Text(String::new()),
            ),
            (
                ProtoWsMessage::Binary(Vec::new()),
                WebSocketMessage::Binary(Vec::new()),
            ),
            (
                ProtoWsMessage::Ping(Vec::new()),
                WebSocketMessage::Ping(Vec::new()),
            ),
            (
                ProtoWsMessage::Pong(Vec::new()),
                WebSocketMessage::Pong(Vec::new()),
            ),
            (
                ProtoWsMessage::Close(ProtoWsClose {
                    code: 1000,
                    reason: String::new(),
                }),
                WebSocketMessage::Close {
                    code: 1000,
                    reason: None,
                },
            ),
        ]
        .map(|(case, exp)| {
            (
                ProtoWebSocket {
                    socket_id: b"test_id".to_vec(),
                    message: Some(case),
                },
                WebSocket {
                    socket_id: Id::try_from(b"test_id".to_vec()).unwrap(),
                    message: exp,
                },
            )
        });

        for (case, exp) in protobuf_msgs {
            assert_eq!(WebSocket::try_from(case).unwrap(), exp);
        }
    }

    #[test]
    fn test_from_websocket() {
        // check all variants
        let proto_msgs = [
            (
                WebSocketMessage::Text(String::new()),
                ProtoWsMessage::Text(String::new()),
            ),
            (
                WebSocketMessage::Binary(Vec::new()),
                ProtoWsMessage::Binary(Vec::new()),
            ),
            (
                WebSocketMessage::Ping(Vec::new()),
                ProtoWsMessage::Ping(Vec::new()),
            ),
            (
                WebSocketMessage::Pong(Vec::new()),
                ProtoWsMessage::Pong(Vec::new()),
            ),
            (
                WebSocketMessage::Close {
                    code: 1000,
                    reason: None,
                },
                ProtoWsMessage::Close(ProtoWsClose {
                    code: 1000,
                    reason: String::new(),
                }),
            ),
        ]
        .map(|(case, exp)| {
            (
                WebSocket {
                    socket_id: Id::try_from(b"test_id".to_vec()).unwrap(),
                    message: case,
                },
                ProtoWebSocket {
                    socket_id: b"test_id".to_vec(),
                    message: Some(exp),
                },
            )
        });

        for (case, exp) in proto_msgs {
            assert_eq!(ProtoWebSocket::from(case), exp);
        }
    }
}
