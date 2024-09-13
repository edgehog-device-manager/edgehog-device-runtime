// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

use edgehog_device_forwarder_proto as proto;
use edgehog_device_forwarder_proto::message::Protocol;
use edgehog_device_forwarder_proto::{
    message::Protocol as ProtobufProtocol, web_socket::Message as ProtobufWsMessage,
    WebSocket as ProtobufWebSocket,
};
use tokio_tungstenite::tungstenite::Message as TungMessage;

use edgehog_device_runtime_forwarder::test_utils::create_ws_msg;
#[cfg(feature = "_test-utils")]
use edgehog_device_runtime_forwarder::test_utils::{
    create_http_upgrade_req, is_ws_upgrade_response, send_ws_and_wait_next, MockWebSocket,
    TestConnections,
};

#[cfg(feature = "_test-utils")]
#[tokio::test]
async fn test_internal_ws() {
    // Mock server for ttyd
    let mut test_connections = TestConnections::<MockWebSocket>::init().await;

    // Edgehog
    let mut ws_bridge = test_connections.mock_ws_server().await;
    // Device listener (waiting for Edgehog -> Device)
    let listener = test_connections.mock_server.device_listener().unwrap();
    let connection_handle = MockWebSocket::open_ws_device(listener);

    let request_id = "3647edbb-6747-4827-a3ef-dbb6239e3326".as_bytes().to_vec();
    let url = format!(
        "ws://localhost:{}/remote-terminal?session=abcd",
        test_connections.mock_server.port().unwrap()
    );
    let http_req = create_http_upgrade_req(request_id.clone(), &url)
        .expect("failed to create http upgrade request");

    let protobuf_res = send_ws_and_wait_next(&mut ws_bridge, http_req).await;

    let (socket_id, protobuf_http) = match protobuf_res.protocol.unwrap() {
        Protocol::Http(http) => (http.request_id, http.message.unwrap()),
        Protocol::Ws(_) => panic!("should be http upgrade response"),
    };

    // check if the response id is the same as the request one
    assert_eq!(request_id, socket_id);
    assert!(is_ws_upgrade_response(protobuf_http));

    // retrieve the ID of the connection and try sending a WebSocket message
    test_connections.mock(connection_handle).await;

    let content = b"data".to_vec();
    let data = TungMessage::Binary(content.clone());
    let ws_binary_msg = create_ws_msg(socket_id.clone(), data);
    let protobuf_res = send_ws_and_wait_next(&mut ws_bridge, ws_binary_msg).await;

    assert_eq!(
        protobuf_res,
        proto::Message {
            protocol: Some(ProtobufProtocol::Ws(ProtobufWebSocket {
                message: Some(ProtobufWsMessage::Binary(content)),
                socket_id: socket_id.clone()
            }))
        }
    );

    // sending ping
    let content = b"ping".to_vec();
    let data = TungMessage::Ping(content.clone());
    let ws_ping_msg = create_ws_msg(socket_id.clone(), data.clone());
    let protobuf_res = send_ws_and_wait_next(&mut ws_bridge, ws_ping_msg).await;

    // check that a pong is received
    assert_eq!(
        protobuf_res,
        proto::Message {
            protocol: Some(ProtobufProtocol::Ws(ProtobufWebSocket {
                message: Some(ProtobufWsMessage::Pong(content)),
                socket_id: socket_id.clone()
            }))
        }
    );
}
