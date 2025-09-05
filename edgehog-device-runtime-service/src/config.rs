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

//! Configuration for the service

use std::net::SocketAddr;
use std::path::PathBuf;

use cfg_if::cfg_if;
use serde::{Deserialize, Serialize};

/// Configuration for the [`EdgehogService`](crate::service::EdgehogService)
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
pub struct Config {
    /// Flag to enable the service
    #[serde(default)]
    pub enabled: bool,
    /// Listener for the service
    #[serde(default)]
    pub listener: Listener,
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
        cfg_if! {
            if #[cfg(unix)] {
                let path = std::env::var("XDG_RUNTIME_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| PathBuf::from("/tmp"))
                    .join("edgehog-device-runtime.sock");

                Listener::Unix(path)
            } else {
                Listener::Socket(std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 50052))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_deserialize_config() {
        let file = r#"
        [listener]
        unix = "/foo"
        "#;

        let exp = Config {
            enabled: false,
            listener: Listener::Unix(PathBuf::from("/foo")),
        };

        let res: Config = toml::from_str(file).unwrap();

        assert_eq!(res, exp);
    }

    #[test]
    fn should_serialize_config() {
        let exp = r#"enabled = true

[listener]
socket = "0.0.0.0:8080"
"#;

        let conf = Config {
            enabled: true,
            listener: Listener::Socket(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::UNSPECIFIED,
                8080,
            ))),
        };

        let res = toml::to_string_pretty(&conf).unwrap();

        assert_eq!(res, exp);
    }
}
