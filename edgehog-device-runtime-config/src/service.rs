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

use crate::Mergiable;

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

impl Mergiable for ServiceConfig {
    fn merge(&mut self, value: Self) {
        let Self { enabled, listener } = value;

        self.enabled = enabled.or(self.enabled);
        self.listener = listener.or(self.listener.take());
    }
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

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    pub(crate) fn mock_service_config() -> Option<ServiceConfig> {
        Some(ServiceConfig {
            enabled: Some(true),
            listener: Some(Listener::Unix(PathBuf::from(
                "/tmp/edgehog-device-runtime.sock",
            ))),
        })
    }

    #[test]
    fn service_config_roundtrip() {
        let exp = mock_service_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ServiceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn service_config_empty_roundtrip() {
        let exp = ServiceConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ServiceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn service_config_socket_roundtrip() {
        let exp = ServiceConfig {
            enabled: Some(true),
            listener: Some(Listener::Socket("127.0.0.1:50052".parse().unwrap())),
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: ServiceConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn listener_default() {
        let default_listener = Listener::default();
        if cfg!(unix) {
            assert!(matches!(default_listener, Listener::Unix(_)));
        } else {
            assert!(matches!(default_listener, Listener::Socket(_)));
        }
    }

    #[test]
    fn service_config_merge() {
        let mut base = ServiceConfig {
            enabled: Some(true),
            listener: Some(Listener::Unix(PathBuf::from("/tmp/base.sock"))),
        };

        let override_cfg = ServiceConfig {
            enabled: None,
            listener: Some(Listener::Socket("127.0.0.1:8080".parse().unwrap())),
        };

        let exp = ServiceConfig {
            enabled: Some(true),
            listener: Some(Listener::Socket("127.0.0.1:8080".parse().unwrap())),
        };

        base.merge(override_cfg);

        assert_eq!(base, exp);
    }

    #[test]
    #[cfg(unix)]
    fn listener_get_default_unix() {
        let root = std::env::var("XDG_RUNTIME_DIR").unwrap_or("/tmp".to_string());
        let mut path = PathBuf::from(root);

        path.push("edgehog-device-runtime.sock");

        let exp = Listener::Unix(path);

        assert_eq!(Listener::default(), exp);
    }
}
