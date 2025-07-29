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

use std::fmt::Display;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use edgehog_containers::local::ContainerHandle;
use edgehog_proto::tonic::transport::server::{Connected, TcpIncoming};
use edgehog_proto::tonic::transport::Server;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use tracing::info;

mod containers;

/// Options for the [`EdgehogService`]
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    /// Listener for the service.
    pub listener: Listener,
}

/// Listener for the service
#[derive(Debug, Clone)]
pub enum Listener {
    /// Unix domain socket
    #[cfg(unix)]
    Unix(PathBuf),
    /// TCP socket
    Socket(SocketAddr),
}

impl Display for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Listener::Unix(path_buf) => {
                write!(f, "{}", path_buf.display())
            }
            Listener::Socket(socket_addr) => {
                write!(f, "{socket_addr}")
            }
        }
    }
}

/// Local service for the Edgehog device runtime
#[derive(Debug)]
pub struct EdgehogService {
    containers: ContainerHandle,
}

impl EdgehogService {
    /// Starts the service with the given options
    pub async fn run(self, options: ServiceOptions, cancel: CancellationToken) -> eyre::Result<()> {
        let this = Arc::new(self);

        info!(listener = %options.listener, "starting edgehog service");

        match options.listener {
            #[cfg(unix)]
            Listener::Unix(path_buf) => {
                let listener = UnixListener::bind(path_buf)?;
                let stream = UnixListenerStream::new(listener);

                serve(this, cancel, stream).await?;
            }
            Listener::Socket(addr) => {
                let listener = TcpIncoming::bind(addr)?;

                serve(this, cancel, listener).await?;
            }
        }

        Ok(())
    }
}

async fn serve<S, IO>(
    this: Arc<EdgehogService>,
    cancel: CancellationToken,
    incoming: S,
) -> eyre::Result<()>
where
    S: Stream<Item = std::io::Result<IO>>,
    IO: AsyncRead + AsyncWrite + Connected + Unpin + Send + 'static,
{
    Server::builder()
        .add_service(
            edgehog_proto::containers::latest::containers_service_server::ContainersServiceServer::from_arc(this),
        )
        .serve_with_incoming_shutdown(incoming, async {
            cancel.cancelled().await;

            info!("shutting down edgehog service")
        }).await?;

    Ok(())
}
