// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Structure to manage the service.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use edgehog_proto::tonic::transport::server::{Connected, TcpIncoming};
use edgehog_proto::tonic::transport::Server;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tokio_stream::Stream;

mod containers;

#[derive(Debug, Clone)]
struct ServiceOptions {
    listener: Listener,
}

#[derive(Debug, Clone)]
enum Listener {
    #[cfg(unix)]
    Unix(PathBuf),
    Socket(SocketAddr),
}

#[derive(Debug, Clone, Copy)]
struct EdgehogService {}

impl EdgehogService {
    async fn spawn(self, options: ServiceOptions) -> eyre::Result<()> {
        let this = Arc::new(self);

        match options.listener {
            #[cfg(unix)]
            Listener::Unix(path_buf) => {
                let listener = UnixListener::bind(path_buf)?;
                let stream = UnixListenerStream::new(listener);

                fun_name(this, stream)
            }
            Listener::Socket(addr) => {
                let listener = TcpIncoming::bind(addr)?;

                fun_name(this, listener)
            }
        }

        Ok(())
    }
}

fn fun_name<S, IO>(this: Arc<EdgehogService>, incoming: S)
where
    S: Stream<Item = std::io::Result<IO>>,
    IO: AsyncRead + AsyncWrite + Connected + Unpin + Send + 'static,
{
    Server::builder().add_service(edgehog_proto::edgehog_deviceruntime_containers_v1::containers_server::ContainersServer::from_arc(Arc::clone(&this))).serve_with_incoming_shutdown(incoming ,async {
        todo!()
    });
}
