// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Configures the Edgehog Local Service.

use std::net::{SocketAddr, SocketAddrV4};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration for the Edgehog Local Service.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ServiceConfig {
    /// Flag to enable the service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Listener for the service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listener: Option<Listener>,
}

/// Listener for the service
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Listener {
    /// Unix domain socket
    Unix(PathBuf),
    /// TCP socket
    Socket(SocketAddr),
}

impl Default for Listener {
    fn default() -> Self {
        if cfg!(unix) {
            let path = std::env::var("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
                .join("edgehog-device-runtime.sock");

            Listener::Unix(path)
        } else {
            Listener::Socket(SocketAddr::V4(SocketAddrV4::new(
                std::net::Ipv4Addr::LOCALHOST,
                50052,
            )))
        }
    }
}
