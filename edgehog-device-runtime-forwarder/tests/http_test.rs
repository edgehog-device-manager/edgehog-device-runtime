// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

use edgehog_device_forwarder_proto::http::Message as HttpMessage;
use edgehog_device_forwarder_proto::{
    http::Request as HttpRequest, http::Response as HttpResponse, message::Protocol, Http,
    Message as ProtoMessage,
};
use futures::{SinkExt, StreamExt};
use httpmock::prelude::*;
use httpmock::Mock;
use std::future::Future;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::{tungstenite::Message as TungMessage, WebSocketStream};
use url::Url;

use edgehog_device_forwarder_proto::prost::Message;
use edgehog_device_runtime_forwarder::connections_manager::{ConnectionsManager, Disconnected};
use uuid::Uuid;

async fn bind_port() -> (TcpListener, u16) {
    let listener = TcpListener::bind("localhost:0")
        .await
        .expect("failed to create a tcp listener");

    let port = listener
        .local_addr()
        .expect("failed to retrieve local addr")
        .port();

    (listener, port)
}

async fn con_manager(url: Url) -> Result<(), Disconnected> {
    let mut con_manager = ConnectionsManager::connect(url)
        .await
        .expect("failed to connect connections manager");
    con_manager.handle_connections().await
}

fn create_http_req(url: &str, request_id: Vec<u8>) -> TungMessage {
    let url = Url::parse(url).expect("failed to pars Url");
    let proto_msg = ProtoMessage {
        protocol: Some(Protocol::Http(Http {
            request_id,
            message: Some(HttpMessage::Request(HttpRequest {
                path: url.path().trim_start_matches('/').to_string(),
                method: http::Method::GET.to_string(),
                query_string: url.query().unwrap_or_default().to_string(),
                port: url.port().expect("nonexistent port").into(),
                ..Default::default()
            })),
        })),
    };

    let mut buf = Vec::with_capacity(proto_msg.encoded_len());
    proto_msg
        .encode(&mut buf)
        .expect("failed to encode protobuf");

    TungMessage::Binary(buf)
}

struct TestConnections {
    mock_server: MockServer,
    listener: TcpListener,
    connections_handle: JoinHandle<Result<(), Disconnected>>,
}

impl TestConnections {
    async fn init() -> Self {
        let mock_server = MockServer::start();

        let (listener, port) = bind_port().await;
        let url = format!("ws://localhost:{port}/remote-terminal?session_token=1234")
            .parse()
            .expect("failed to parse url");

        Self {
            mock_server,
            listener,
            connections_handle: tokio::spawn(con_manager(url)),
        }
    }

    fn endpoint(&self) -> Mock {
        // Create a mock on the server.
        self.mock_server.mock(|when, then| {
            when.method(GET)
                .path("/remote-terminal")
                .query_param("session_token", "abcd");
            then.status(200)
                .header("content-type", "text/html")
                .body("just do it");
        })
    }

    async fn mock_ws_server<F>(&self, f: impl FnOnce(WebSocketStream<TcpStream>) -> F)
    where
        F: Future,
    {
        let (stream, _) = self
            .listener
            .accept()
            .await
            .expect("failed to accept connection");

        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .expect("failed to open a ws with the device");

        f(ws_stream).await;
    }

    async fn assert<'a>(self) {
        let res = self.connections_handle.await.expect("task join failed");
        assert!(res.is_ok(), "connection manager error {}", res.unwrap_err());
    }
}

#[tokio::test]
async fn test_connect() {
    let test_connections = TestConnections::init().await;
    let endpoint = test_connections.endpoint();

    let test_url = test_connections
        .mock_server
        .url("/remote-terminal?session_token=abcd");

    test_connections
        .mock_ws_server(|mut ws| async move {
            let request_id = Uuid::new_v4().as_bytes().to_vec();
            let http_req = create_http_req(&test_url, request_id);

            ws.send(http_req.clone())
                .await
                .expect("failed to send over ws");

            // send again another request with the same ID. This should cause an IdAlreadyUsed error
            // which is internally handled (print in debug mode)
            ws.send(http_req).await.expect("failed to send over ws");

            // the 1st request is correctly handled
            let http_res = ws
                .next()
                .await
                .expect("ws already closed")
                .expect("failed to receive from ws")
                .into_data();

            let proto_res = ProtoMessage::decode(http_res.as_slice())
                .expect("failed to decode tung message into protobuf");

            assert!(matches!(
                proto_res,
                ProtoMessage {
                    protocol: Some(Protocol::Http(Http {
                        message: Some(HttpMessage::Response(HttpResponse {
                            status_code: 200,
                            ..
                        })),
                        ..
                    })),
                }
            ));

            ws.close(None).await.expect("failed to close ws");
        })
        .await;

    endpoint.assert();
    test_connections.assert().await;
}
