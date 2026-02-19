// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
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

use cfg_if::cfg_if;
use edgehog_proto::tonic::transport::server::{Connected, TcpIncoming};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::UnixListener;
use tokio_stream::Stream;
use tokio_stream::wrappers::UnixListenerStream;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::Config;

#[cfg(feature = "containers")]
mod containers;

/// Options for the [`EdgehogService`]
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    /// Listener for the service.
    pub listener: Listener,
}

impl TryFrom<Config> for ServiceOptions {
    type Error = eyre::Report;

    fn try_from(value: Config) -> Result<Self, Self::Error> {
        Ok(ServiceOptions {
            listener: value.listener.try_into()?,
        })
    }
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

impl TryFrom<crate::config::Listener> for Listener {
    type Error = eyre::Report;

    fn try_from(value: crate::config::Listener) -> Result<Self, Self::Error> {
        match value {
            crate::config::Listener::Unix(path_buf) => {
                cfg_if! {
                    if #[cfg(unix)] {
                        Ok(Listener::Unix(path_buf))
                    } else {
                        Err(eyre!("target doesn't support unix socket: {}", path_buf.display()))
                    }
                }
            }
            crate::config::Listener::Socket(socket_addr) => Ok(Listener::Socket(socket_addr)),
        }
    }
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
    options: ServiceOptions,
    #[cfg(feature = "containers")]
    containers: self::containers::SharedContainerHandle,
}

impl EdgehogService {
    /// Create a new instance of the service
    pub fn new(
        options: ServiceOptions,
        #[cfg(feature = "containers")] containers: self::containers::SharedContainerHandle,
    ) -> Self {
        Self {
            options,
            #[cfg(feature = "containers")]
            containers,
        }
    }

    /// Starts the service with the given options
    pub async fn run(self, cancel: CancellationToken) -> eyre::Result<()> {
        let this = Arc::new(self);

        info!(listener = %this.options.listener, "starting edgehog service");

        match &this.options.listener {
            #[cfg(unix)]
            Listener::Unix(path_buf) => {
                if let Some(path) = path_buf.parent() {
                    tokio::fs::create_dir_all(path).await?;
                }

                // remove the old file
                if path_buf.exists() {
                    tokio::fs::remove_file(&path_buf).await?;
                }

                let listener = UnixListener::bind(path_buf)?;
                let stream = UnixListenerStream::new(listener);

                serve(this, cancel, stream).await?;
            }
            Listener::Socket(addr) => {
                let listener = TcpIncoming::bind(*addr)?;

                serve(this, cancel, listener).await?;
            }
        }

        Ok(())
    }
}

impl Drop for EdgehogService {
    fn drop(&mut self) {
        match &self.options.listener {
            #[cfg(unix)]
            Listener::Unix(path_buf) => {
                if path_buf.exists()
                    && let Err(err) = std::fs::remove_file(path_buf)
                {
                    tracing::error!(
                        error = format!("{:#}", eyre::Report::new(err)),
                        "couldn't remove unix socket"
                    );
                }
            }
            Listener::Socket(_) => {}
        }
    }
}

cfg_if! {
    if #[cfg(feature = "containers")] {
        async fn serve<S, IO>(
            this: Arc<EdgehogService>,
            cancel: CancellationToken,
            incoming: S,
        ) -> eyre::Result<()>
        where
            S: Stream<Item = std::io::Result<IO>>,
            IO: AsyncRead + AsyncWrite + Connected + Unpin + Send + 'static,
        {
            use edgehog_proto::tonic::transport::Server;

            let mut builder = Server::builder();

            builder
                .add_service(edgehog_proto::containers::latest::containers_service_server::ContainersServiceServer::from_arc(this))
                .serve_with_incoming_shutdown(incoming, async {
                    cancel.cancelled().await;

                    info!("shutting down edgehog service")
                })
                .await?;

            Ok(())
        }
    } else {
        async fn serve<S, IO>(
            _this: Arc<EdgehogService>,
            _cancel: CancellationToken,
            _incoming: S,
        ) -> eyre::Result<()>
        where
            S: Stream<Item = std::io::Result<IO>>,
            IO: AsyncRead + AsyncWrite + Connected + Unpin + Send + 'static,
        {
            unimplemented!("no service implemented");
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {

    #[cfg(feature = "containers")]
    use edgehog_containers::local::MockContainerHandle;

    use super::*;

    pub(crate) fn mock_service(
        #[cfg(feature = "containers")] handle: MockContainerHandle,
    ) -> EdgehogService {
        EdgehogService {
            options: ServiceOptions {
                listener: Listener::Socket("0.0.0.0:0".parse().unwrap()),
            },
            #[cfg(feature = "containers")]
            containers: Arc::new(tokio::sync::OnceCell::const_new_with(handle)),
        }
    }
}
