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

use astarte_device_sdk::builder::DeviceBuilder;
use astarte_device_sdk::introspection::AddInterfaceError;
use astarte_device_sdk::prelude::*;
use astarte_device_sdk::store::SqliteStore;
use astarte_device_sdk::transport::grpc::{GrpcConfig, GrpcError};
use astarte_device_sdk::DeviceClient;
use astarte_device_sdk::Error as AstarteError;
use serde::Deserialize;
use std::path::Path;
use tokio::task::JoinHandle;
use uuid::{uuid, Uuid};

/// Device runtime node identifier.
const DEVICE_RUNTIME_NODE_UUID: Uuid = uuid!("d72a6187-7cf1-44cc-87e8-e991936166db");

/// Error returned by the [`astarte_device_sdk`].
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum MessageHubError {
    /// missing configuration for the Astarte Message Hub
    MissingConfig,
    /// couldn't add interfaces directory
    Interfaces(#[source] AddInterfaceError),
    /// couldn't connect to Astarte
    Connect(#[source] astarte_device_sdk::Error),
    /// Invalid endpoint
    Endpoint(#[source] GrpcError),
}

/// Struct containing the configuration options for the Astarte message hub.
#[derive(Debug, Deserialize, Clone)]
pub struct AstarteMessageHubOptions {
    /// The Endpoint of the Astarte Message Hub
    endpoint: String,
}

impl AstarteMessageHubOptions {
    pub async fn connect<P>(
        &self,
        store: SqliteStore,
        interface_dir: P,
    ) -> Result<
        (
            DeviceClient<SqliteStore>,
            JoinHandle<Result<(), AstarteError>>,
        ),
        MessageHubError,
    >
    where
        P: AsRef<Path>,
    {
        let grpc_cfg = GrpcConfig::from_url(DEVICE_RUNTIME_NODE_UUID, self.endpoint.clone())
            .map_err(MessageHubError::Endpoint)?;

        let (device, mut connection) = DeviceBuilder::new()
            .store(store)
            .interface_directory(interface_dir)
            .map_err(MessageHubError::Interfaces)?
            .connect(grpc_cfg)
            .await
            .map_err(MessageHubError::Connect)?
            .build();

        let handle = tokio::spawn(async move { connection.handle_events().await });

        Ok((device, handle))
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::types::AstarteType;
    use astarte_device_sdk::AstarteAggregate;
    use astarte_message_hub_proto::astarte_message::Payload;
    use astarte_message_hub_proto::message_hub_server::{MessageHub, MessageHubServer};
    use astarte_message_hub_proto::tonic::{Code, Request, Response, Status};
    use astarte_message_hub_proto::AstarteMessage;
    use async_trait::async_trait;
    use std::net::{Ipv6Addr, SocketAddr};
    use tokio::sync::oneshot::Sender;
    use tokio::task::JoinHandle;

    use crate::data::astarte_message_hub_node::AstarteMessageHubOptions;
    use crate::data::tests::create_tmp_store;
    use crate::data::{Publisher, Subscriber};

    mockall::mock! {
        MsgHub {}
        #[async_trait]
        impl MessageHub for MsgHub{
            type AttachStream = tokio_stream::wrappers::ReceiverStream<Result<astarte_message_hub_proto::AstarteMessage, Status>>;
            async fn attach(
                &self,
                request: Request<astarte_message_hub_proto::Node>,
            ) -> Result<Response<<Self as MessageHub>::AttachStream>, Status> ;
            async fn send(
                &self,
                request: Request<astarte_message_hub_proto::AstarteMessage>,
            ) -> Result<Response<pbjson_types::Empty>, Status> ;
            async fn detach(
                &self,
                request: Request<astarte_message_hub_proto::Node>,
            ) -> Result<Response<pbjson_types::Empty>, Status> ;
        }
    }

    async fn run_local_server(msg_hub: MockMsgHub) -> (JoinHandle<()>, Sender<()>, u16) {
        let (drop_tx, drop_rx) = tokio::sync::oneshot::channel::<()>();
        // Ask the OS to bind on an available port
        let addr: SocketAddr = (Ipv6Addr::LOCALHOST, 0).into();
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .expect("failed to bind port");
        let addr = listener.local_addr().expect("failed to get local address");

        let handle = tokio::spawn(async move {
            astarte_message_hub_proto::tonic::transport::Server::builder()
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

    #[tokio::test]
    async fn attach_connect_fail() {
        let msg_hub = MockMsgHub::new();

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let opts = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        };

        let (store, tmp_store_path) = create_tmp_store().await;

        let node_result = opts.connect(store, &tmp_store_path).await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");
        assert!(node_result.is_err());
    }

    #[tokio::test]
    async fn attach_node_fail() {
        let mut msg_hub = MockMsgHub::new();

        msg_hub
            .expect_attach()
            .returning(|_| Err(Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let (store, tmp_store_path) = create_tmp_store().await;

        let node_result = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_store_path)
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

        let (store, tmp_store_path) = create_tmp_store().await;

        let node_result = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_store_path)
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
            .returning(|_| Err(Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let (store, tmp_store_path) = create_tmp_store().await;

        let (pub_sub, _handle) = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_store_path)
        .await
        .unwrap();

        let send_result = pub_sub
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
        let (store, tmp_dir) = create_tmp_store().await;

        tokio::fs::write(
            tmp_dir.path().join("test.Individual.json"),
            r#"{"interface_name": "test.Individual",
    "version_major": 0,
    "version_minor": 1,
    "type": "datastream",
    "ownership": "device",
    "mappings": [
        {
            "endpoint": "/sValue",
            "type": "string"
        }
    ]
}"#,
        )
        .await
        .unwrap();

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

        let (pub_sub, _handle) = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_dir)
        .await
        .unwrap();

        let send_result = pub_sub
            .send(
                "test.Individual",
                "/sValue",
                AstarteType::String("value".to_string()),
            )
            .await;
        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(send_result.is_ok(), "error {}", send_result.unwrap_err());
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
            .returning(|_| Err(Status::new(Code::Internal, "".to_string())));

        let (server_handle, drop_sender, port) = run_local_server(msg_hub).await;

        let (store, tmp_store_path) = create_tmp_store().await;

        let (pub_sub, _handle) = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_store_path)
        .await
        .unwrap();

        #[derive(AstarteAggregate)]
        struct Obj {
            s_value: String,
        }

        let send_result = pub_sub
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
        let (store, tmp_dir) = create_tmp_store().await;

        tokio::fs::write(
            tmp_dir.path().join("test.Object.json"),
            r#"{"interface_name": "test.Object",
    "version_major": 0,
    "version_minor": 1,
    "type": "datastream",
    "aggregation": "object",
    "ownership": "device",
    "mappings": [{
            "endpoint": "/obj/s_value",
            "type": "string"
}]}"#,
        )
        .await
        .unwrap();

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

        let (pub_sub, _handle) = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_dir)
        .await
        .unwrap();

        #[derive(AstarteAggregate)]
        struct Obj {
            s_value: String,
        }

        let send_result = pub_sub
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

        assert!(send_result.is_ok(), "error {}", send_result.unwrap_err());
    }

    #[tokio::test]
    async fn receive_success() {
        let (store, tmp_dir) = create_tmp_store().await;

        tokio::fs::write(
            tmp_dir.path().join("test.server.Value.json"),
            r#"{"interface_name": "test.server.Value",
    "version_major": 0,
    "version_minor": 1,
    "type": "datastream",
    "ownership": "server",
    "mappings": [
        {
            "endpoint": "/req",
            "type": "integer"
        }
    ]
}"#,
        )
        .await
        .unwrap();

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

        let (pub_sub, _handle) = AstarteMessageHubOptions {
            endpoint: format!("http://[::1]:{port}"),
        }
        .connect(store, &tmp_dir)
        .await
        .unwrap();

        let data_receive_result = pub_sub.recv().await;

        drop_sender.send(()).expect("send shutdown");
        server_handle.await.expect("server shutdown");

        assert!(
            data_receive_result.is_ok(),
            "error encountered {}",
            data_receive_result.unwrap_err()
        );
    }
}
