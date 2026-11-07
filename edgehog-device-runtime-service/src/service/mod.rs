// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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
#[cfg(unix)]
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use cfg_if::cfg_if;
use edgehog_proto::tonic::transport::server::{Connected, TcpIncoming};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::Stream;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::ServiceArgs;

use self::config::ConfigState;

pub mod config;
#[cfg(feature = "containers")]
mod containers;

/// Options for the [`EdgehogService`]
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    /// Listener for the service.
    pub listener: Listener,
}

impl TryFrom<ServiceArgs> for ServiceOptions {
    type Error = eyre::Report;

    fn try_from(value: ServiceArgs) -> Result<Self, Self::Error> {
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
    Unix(std::path::PathBuf),
    /// TCP socket
    Socket(SocketAddr),
}

impl TryFrom<edgehog_config::service::Listener> for Listener {
    type Error = eyre::Report;

    fn try_from(value: edgehog_config::service::Listener) -> Result<Self, Self::Error> {
        match value {
            edgehog_config::service::Listener::Unix(path_buf) => {
                cfg_if! {
                    if #[cfg(unix)] {
                        Ok(Listener::Unix(path_buf))
                    } else {
                        Err(eyre::eyre!("target doesn't support unix socket: {}", path_buf.display()))
                    }
                }
            }
            edgehog_config::service::Listener::Socket(socket_addr) => {
                Ok(Listener::Socket(socket_addr))
            }
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
    config: ConfigState,
    #[cfg(feature = "containers")]
    containers: self::containers::SharedContainerHandle,
}

impl EdgehogService {
    /// Create a new instance of the service
    pub fn new(options: ServiceOptions, config: ConfigState) -> Self {
        Self {
            options,
            config,
            #[cfg(feature = "containers")]
            containers: self::containers::SharedContainerHandle::default(),
        }
    }

    /// Sets the container handle
    #[cfg(feature = "containers")]
    pub fn set_container_handle(&mut self, containers: self::containers::SharedContainerHandle) {
        self.containers = containers;
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

                let listener = tokio::net::UnixListener::bind(path_buf)?;
                let stream = tokio_stream::wrappers::UnixListenerStream::new(listener);

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
            Listener::Unix(path_buf) => match std::fs::remove_file(path_buf) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::error!(
                        error = format!("{:#}", eyre::Report::new(err)),
                        "couldn't remove unix socket"
                    );
                }
            },
            Listener::Socket(_) => {}
        }
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
    use edgehog_proto::tonic::transport::Server;

    let mut builder = Server::builder();

    #[cfg(feature = "containers")]
    let builder = builder.add_service(
        edgehog_proto::containers::latest::containers_service_server::ContainersServiceServer::from_arc(
            this.clone(),
        ),
    );

    builder
        .add_service(
            edgehog_proto::config::v1::config_service_server::ConfigServiceServer::from_arc(this),
        )
        .serve_with_incoming_shutdown(incoming, async {
            cancel.cancelled().await;

            info!("shutting down edgehog service")
        })
        .await?;

    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {

    use std::path::PathBuf;

    #[cfg(feature = "containers")]
    use tokio::sync::OnceCell;

    use super::*;

    #[cfg_attr(not(feature = "containers"), expect(dead_code))]
    pub(crate) fn mock_service(
        #[cfg(feature = "containers")] handle: edgehog_containers::local::MockContainerHandle,
    ) -> EdgehogService {
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let config = ConfigState::new(PathBuf::from("/dev/null"), tx);

        EdgehogService {
            config,
            options: ServiceOptions {
                listener: Listener::Socket("0.0.0.0:0".parse().unwrap()),
            },
            #[cfg(feature = "containers")]
            containers: Arc::new(OnceCell::new_with(Some(handle))),
        }
    }

    pub(crate) fn mock_service_with_config(config: ConfigState) -> EdgehogService {
        EdgehogService {
            config,
            options: ServiceOptions {
                listener: Listener::Socket("0.0.0.0:0".parse().unwrap()),
            },
            #[cfg(feature = "containers")]
            containers: Arc::new(OnceCell::new()),
        }
    }
}
