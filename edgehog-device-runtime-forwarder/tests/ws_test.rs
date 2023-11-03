// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

use edgehog_device_forwarder_proto as proto;
use edgehog_device_forwarder_proto::message::Protocol;
use edgehog_device_forwarder_proto::{
    message::Protocol as ProtobufProtocol, web_socket::Message as ProtobufWsMessage,
    WebSocket as ProtobufWebSocket,
};

#[cfg(feature = "_test-utils")]
use edgehog_device_runtime_forwarder::test_utils::{
    create_http_upgrade_req, create_ws_binary, is_ws_upgrade_response, send_ws_and_wait_next,
    MockWebSocket, TestConnections,
};

#[cfg(feature = "_test-utils")]
#[tokio::test]
async fn test_internal_ws() {
    use uuid::Uuid;

    let mut test_connections = TestConnections::<MockWebSocket>::init().await;

    let mut ws_bridge = test_connections.mock_ws_server().await;
    let listener = test_connections.mock_server.device_listener().unwrap();
    let connection_handle = MockWebSocket::open_ws_device(listener);

    let request_id = Uuid::new_v4().as_bytes().to_vec();
    let url = format!(
        "ws://localhost:{}/remote-terminal?session_token=abcd",
        test_connections.mock_server.port().unwrap()
    );
    let http_req = create_http_upgrade_req(request_id.clone(), &url);

    let protobuf_res = send_ws_and_wait_next(&mut ws_bridge, http_req).await;

    let (socket_id, protobuf_http) = match protobuf_res.protocol.unwrap() {
        Protocol::Http(http) => (http.request_id, http.message.unwrap()),
        Protocol::Ws(_) => panic!("should be http upgrade response"),
    };

    // check if the response id is the same as the request one
    assert_eq!(request_id, socket_id);
    assert!(is_ws_upgrade_response(protobuf_http));

    // retrieve the ID of the connection and try sending a websocket message
    test_connections.mock(connection_handle).await;

    let data = b"data".to_vec();
    let ws_msg = create_ws_binary(socket_id.clone(), data.clone());

    let protobuf_res = send_ws_and_wait_next(&mut ws_bridge, ws_msg).await;

    assert_eq!(
        protobuf_res,
        proto::Message {
            protocol: Some(ProtobufProtocol::Ws(ProtobufWebSocket {
                message: Some(ProtobufWsMessage::Binary(data)),
                socket_id
            }))
        }
    );
}
