/*
 * This file is part of Edgehog.
 *
 * Copyright 2023 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

//! Contains the implementation for the Astarte message hub node.

use astarte_message_hub::proto_message_hub;
use async_trait::async_trait;
use log::warn;
use serde::Deserialize;
use tokio::time::Duration;
use tonic::Response as TonicResponse;
use tonic::Streaming as TonicStreaming;

use crate::data::{Publisher, Subscriber};
use crate::error::DeviceManagerError;

/// Device runtime node identifier.
const DEVICE_RUNTIME_NODE_UUID: &str = "d72a6187-7cf1-44cc-87e8-e991936166db";

/// Struct containing the configuration options for the Astarte message hub.
#[derive(Debug, Deserialize, Clone)]
pub struct AstarteMessageHubOptions {
    /// The Endpoint of the Astarte Message Hub
    endpoint: String,
}

/// Shorthand for the transmit Astarte data event in [`run_node`] function.
type AstarteDataEventSender = tokio::sync::oneshot::Sender<
    Result<astarte_device_sdk::AstarteDeviceDataEvent, astarte_device_sdk::AstarteError>,
>;

/// Main struct for the Astarte message hub node.
/// Contains the communication with Astarte Message Hub.
#[derive(Clone)]
pub struct AstarteMessageHubNode {
    transport_channel: tonic::transport::Channel,
    sender: tokio::sync::mpsc::Sender<AstarteDataEventSender>,
}

impl AstarteMessageHubNode {
    pub async fn new(
        options: AstarteMessageHubOptions,
        interfaces_directory: String,
    ) -> Result<Self, DeviceManagerError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);

        use proto_message_hub::message_hub_client::MessageHubClient;
        let channel = tonic::transport::Channel::from_shared(options.endpoint)
            .map_err(|err| DeviceManagerError::FatalError(err.to_string()))?
            .http2_keep_alive_interval(Duration::from_secs(5))
            .connect()
            .await?;

        let mut message_hub_client = MessageHubClient::new(channel.clone());
        let interface_json = read_interfaces_from_directory(&interfaces_directory)?;

        let node = proto_message_hub::Node {
            uuid: DEVICE_RUNTIME_NODE_UUID.to_string(),
            interface_jsons: interface_json,
        };

        let stream_channel = message_hub_client
            .attach(tonic::Request::new(node))
            .await?
            .into_inner();

        let channel_clone = channel.clone();
        tokio::spawn(run_node(
            interfaces_directory,
            channel_clone,
            stream_channel,
            receiver,
        ));

        Ok(Self {
            transport_channel: channel,
            sender,
        })
    }

    ///Send an Astarte message payload to the Astarte Message Hub.
    async fn send_payload(
        &self,
        interface_name: &str,
        interface_path: &str,
        payload: proto_message_hub::astarte_message::Payload,
    ) -> Result<(), astarte_device_sdk::AstarteError> {
        use proto_message_hub::message_hub_client::MessageHubClient;

        let astarte_message = proto_message_hub::AstarteMessage {
            interface_name: interface_name.to_string(),
            path: interface_path.to_string(),
            timestamp: None,
            payload: Some(payload),
        };

        let mut message_hub_client = MessageHubClient::new(self.transport_channel.clone());
        message_hub_client
            .send(tonic::Request::new(astarte_message))
            .await
            .map_err(|err| {
                astarte_device_sdk::AstarteError::SendError(err.message().to_string())
            })?;

        Ok(())
    }
}

#[async_trait]
impl Publisher for AstarteMessageHubNode {
    async fn send_object<T: 'static>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), astarte_device_sdk::AstarteError>
    where
        T: astarte_device_sdk::AstarteAggregate + Send,
    {
        let payload: proto_message_hub::astarte_message::Payload = data
            .astarte_aggregate()?
            .try_into()
            .map_err(|_| astarte_device_sdk::AstarteError::Conversion)?;

        self.send_payload(interface_name, interface_path, payload)
            .await
    }

    async fn send(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: astarte_device_sdk::types::AstarteType,
    ) -> Result<(), astarte_device_sdk::AstarteError> {
        use astarte_message_hub::proto_message_hub::astarte_message::Payload;

        let payload: Payload = data
            .try_into()
            .map_err(|_| astarte_device_sdk::AstarteError::Conversion)?;

        self.send_payload(interface_name, interface_path, payload)
            .await
    }
}

#[async_trait]
impl Subscriber for AstarteMessageHubNode {
    async fn on_event(
        &mut self,
    ) -> Result<astarte_device_sdk::AstarteDeviceDataEvent, astarte_device_sdk::AstarteError> {
        let (astarte_data_event_tx, astarte_data_event_rx) = tokio::sync::oneshot::channel();

        self.sender.send(astarte_data_event_tx).await.map_err(|_| {
            astarte_device_sdk::AstarteError::SendError("Sender dropped".to_string())
        })?;

        astarte_data_event_rx.await.map_err(|_| {
            astarte_device_sdk::AstarteError::SendError("Unable to receive message".to_string())
        })?
    }
}

/// Runner function for the AstarteMessageHubNode.
///
/// This function receives message from Astarte Message hub and forward it to the device runtime.
/// If this function receives an Error with Status [`tonic::Code::Unavailable`] or [`tonic::Code::Unknown`]
/// it will retry to attach the node to Astarte Message Hub.
pub async fn run_node(
    path: String,
    transport_channel: tonic::transport::Channel,
    mut stream_node_event: tonic::Streaming<proto_message_hub::AstarteMessage>,
    mut receiver: tokio::sync::mpsc::Receiver<AstarteDataEventSender>,
) {
    while let Some(respond_to) = receiver.recv().await {
        let astarte_data_result = match stream_node_event.message().await {
            Ok(option_message) => option_message
                .ok_or_else(|| {
                    astarte_device_sdk::AstarteError::ReceiveError(
                        "Empty message received".to_string(),
                    )
                })
                .and_then(|astarte_message| {
                    astarte_device_sdk::AstarteDeviceDataEvent::try_from(astarte_message)
                        .map_err(|_| astarte_device_sdk::AstarteError::Conversion)
                }),
            Err(status)
                if (status.code() == tonic::Code::Unavailable
                    || status.code() == tonic::Code::Unknown) =>
            {
                // Server shutting down or Server side application throws an exception
                stream_node_event = retry_to_attach_node(&path, transport_channel.clone())
                    .await
                    .into_inner();

                Err(astarte_device_sdk::AstarteError::ReceiveError(
                    "Error during receive message from Astarte Message Hub, Server Unavailable"
                        .to_string(),
                ))
            }
            _ => Err(astarte_device_sdk::AstarteError::ReceiveError(
                "Error during receive message from Astarte Message Hub".to_string(),
            )),
        };

        if respond_to.send(astarte_data_result).is_err() {
            warn!("respond_sender dropped before reply")
        }
    }
}

/// Retries to attach the node to the Astarte Message Hub.
async fn retry_to_attach_node(
    interface_path: &str,
    transport_channel: tonic::transport::Channel,
) -> TonicResponse<TonicStreaming<proto_message_hub::AstarteMessage>> {
    use proto_message_hub::message_hub_client::MessageHubClient;

    let interface_json = read_interfaces_from_directory(interface_path).unwrap();

    let node = proto_message_hub::Node {
        uuid: DEVICE_RUNTIME_NODE_UUID.to_string(),
        interface_jsons: interface_json,
    };

    let mut attach_node_result = MessageHubClient::new(transport_channel.clone())
        .attach(tonic::Request::new(node.clone()))
        .await;

    let mut interval = tokio::time::interval(Duration::from_secs(5));

    while attach_node_result.is_err() {
        interval.tick().await;
        attach_node_result = MessageHubClient::new(transport_channel.clone())
            .attach(tonic::Request::new(node.clone()))
            .await
    }

    attach_node_result.unwrap()
}

fn read_interfaces_from_directory(
    interfaces_directory: &str,
) -> Result<Vec<Vec<u8>>, DeviceManagerError> {
    use std::ffi::OsStr;
    use std::path::Path;

    let interface_files = std::fs::read_dir(Path::new(interfaces_directory))?;
    let json_extension = OsStr::new("json");
    Ok(interface_files
        .filter_map(Result::ok)
        .filter(|f| {
            f.path()
                .extension()
                .map_or_else(|| false, |ext| ext == json_extension)
        })
        .map(|it| std::fs::read(it.path()))
        .filter_map(Result::ok)
        .collect::<Vec<Vec<u8>>>())
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::types::AstarteType;
    use astarte_device_sdk::AstarteAggregate;
    use astarte_message_hub::proto_message_hub::astarte_message::Payload;
    use astarte_message_hub::proto_message_hub::AstarteMessage;
    use std::net::{Ipv6Addr, SocketAddr};
    use tempdir::TempDir;
    use tokio::sync::oneshot::Sender;
    use tokio::task::JoinHandle;
    use tonic::{Code, Request, Response, Status};

    use crate::data::astarte_message_hub_node::{
        read_interfaces_from_directory, AstarteMessageHubNode, AstarteMessageHubOptions,
    };
    use crate::data::{Publisher, Subscriber};

    mockall::mock! {
        MsgHub {}
        #[tonic::async_trait]
        impl astarte_message_hub::proto_message_hub::message_hub_server::MessageHub for MsgHub{
            type AttachStream = tokio_stream::wrappers::ReceiverStream<Result<astarte_message_hub::proto_message_hub::AstarteMessage, Status>>;
            pub async fn attach(
                &self,
                request: Request<astarte_message_hub::proto_message_hub::Node>,
            ) -> Result<Response<<Self as astarte_message_hub::proto_message_hub::message_hub_server::MessageHub>::AttachStream>, Status> ;
            pub async fn send(
                &self,
                request: Request<astarte_message_hub::proto_message_hub::AstarteMessage>,
            ) -> Result<Response<pbjson_types::Empty>, Status> ;
            pub async fn detach(
                &self,
                request: Request<astarte_message_hub::proto_message_hub::Node>,
            ) -> Result<Response<pbjson_types::Empty>, Status> ;
        }
    }

    async fn run_local_server(msg_hub: MockMsgHub) -> (JoinHandle<()>, Sender<()>, u16) {
        use crate::data::astarte_message_hub_node::proto_message_hub::message_hub_server::MessageHubServer;

        let (drop_tx, drop_rx) = tokio::sync::oneshot::channel::<()>();
        // Ask the OS to bind on an available port
        let addr: SocketAddr = (Ipv6Addr::LOCALHOST, 0).into();
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("failed to bind port");
        let addr = listener.local_addr().expect("failed to get local address");

        let handle = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(MessageHubServer::new(msg_hub))
                .serve_with_incoming_shutdown(
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async { drop_rx.await.unwrap() },
                )
                .await
                .unwrap();
        });

        (handle, drop_tx, addr.port())
    }

    #[test]
    fn read_interfaces_from_directory_empty() {
        let dir = TempDir::new("edgehog").unwrap();
        let t_dir = dir.path().to_str().unwrap();
        let read_result = read_interfaces_from_directory(t_dir);

        assert!(read_result.is_ok());
        assert!(read_result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn read_interfaces_from_directory_1_interface() {
        let dir = TempDir::new("edgehog").unwrap();
        let t_dir = dir.path().to_str().unwrap();

        const SERV_PROPS_IFACE: &str = r#"
        {
            "interface_name": "org.astarte-platform.test.test",
            "version_major": 1,
            "version_minor": 1,
            "type": "properties",
            "ownership": "server",
            "mappings": [
                {
                    "endpoint": "/button",
                    "type": "boolean",
                    "explicit_timestamp": true
                },
            ]
        }
        "#;

        tokio::fs::write(
            format!("{}/org.astarte-platform.test.test.json", t_dir),
            SERV_PROPS_IFACE,
        )
        .await
        .expect("Unable to write interface file");

        let read_result = read_interfaces_from_directory(t_dir);

        assert!(read_result.is_ok());
        let interfaces = read_result.unwrap();
        assert_eq!(interfaces.len(), 1);
    }

    #[tokio::test]
    async fn attach_connect_fail() {
        let msg_hub = MockMsgHub::new();

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");
        assert!(node_result.is_err());
    }

    #[tokio::test]
    async fn attach_node_fail() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub
            .expect_attach()
            .returning(|_| Err(tonic::Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");
        assert!(node_result.is_err());
    }

    #[tokio::test]
    async fn attach_success() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");
        assert!(node_result.is_ok());
    }

    #[tokio::test]
    async fn send_fail() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        msg_hub
            .expect_send()
            .returning(|_| Err(tonic::Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        assert!(node_result.is_ok());
        let node = node_result.unwrap();

        let send_result = node
            .send(
                "test.Individual",
                "/sValue",
                AstarteType::String("value".to_string()),
            )
            .await;
        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(send_result.is_err());
    }

    #[tokio::test]
    async fn send_success() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        msg_hub
            .expect_send()
            .returning(|_| Ok(Response::new(pbjson_types::Empty {})));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        assert!(node_result.is_ok());
        let node = node_result.unwrap();

        let send_result = node
            .send(
                "test.Individual",
                "/sValue",
                AstarteType::String("value".to_string()),
            )
            .await;
        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(send_result.is_ok());
    }

    #[tokio::test]
    async fn send_object_fail() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        msg_hub
            .expect_send()
            .returning(|_| Err(tonic::Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        assert!(node_result.is_ok());
        let node = node_result.unwrap();

        #[derive(AstarteAggregate)]
        struct Obj {
            s_value: String,
        }

        let send_result = node
            .send_object(
                "test.Object",
                "/obj",
                Obj {
                    s_value: "test".to_string(),
                },
            )
            .await;
        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(send_result.is_err());
    }

    #[tokio::test]
    async fn send_object_success() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (_, rx) = tokio::sync::mpsc::channel(1);
            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        msg_hub
            .expect_send()
            .returning(|_| Ok(Response::new(pbjson_types::Empty {})));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        assert!(node_result.is_ok());
        let node = node_result.unwrap();

        #[derive(AstarteAggregate)]
        struct Obj {
            s_value: String,
        }

        let send_result = node
            .send_object(
                "test.Object",
                "/obj",
                Obj {
                    s_value: "test".to_string(),
                },
            )
            .await;
        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(send_result.is_ok());
    }

    #[tokio::test]
    async fn receive_success() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub.expect_attach().returning(|_| {
            let (tx, rx) = tokio::sync::mpsc::channel(1);

            tokio::spawn(async move {
                tx.send(Ok(AstarteMessage {
                    interface_name: "test.server.Value".to_string(),
                    path: "/req".to_string(),
                    timestamp: None,
                    payload: Some(Payload::AstarteData(5.into())),
                }))
                .await
                .expect("send astarte message");
            });

            Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
                rx,
            )))
        });

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let node_result = AstarteMessageHubNode::new(
            AstarteMessageHubOptions {
                endpoint: format!("http://[::1]:{port}"),
            },
            "/tmp".to_string(),
        )
        .await;

        assert!(node_result.is_ok());
        let mut node = node_result.unwrap();

        let data_receive_result = node.on_event().await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(data_receive_result.is_ok());
    }
}
