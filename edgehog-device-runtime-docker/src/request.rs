// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Handles Docker request from Astarte.

use std::num::ParseIntError;

use astarte_device_sdk::{event::FromEventError, from_event, DeviceEvent, FromEvent};

use itertools::Itertools;
use tracing::{instrument, trace};

use crate::{
    container::{Binding, Container},
    network::Network,
    properties::container::map_port_bindings,
    service::{ContainerNode, Id},
    volume::Volume,
};

/// Error from handling the Astarte request.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ReqError {
    /// couldn't parse option, expected key=value but got {0}
    Option(String),
    /// couldn't parse container restart policy: {0}
    RestartPolicy(String),
    /// couldn't parse port binding
    PortBinding(#[from] BindingError),
}

/// Create request from Astarte.
#[derive(Debug, Clone)]
pub enum CreateRequests {
    /// Request to pull a [`Image`](crate::image::Image).
    Image(CreateImage),
    /// Request to create a [`Volume`].
    Volume(CreateVolume),
    /// Request to create a [`Network`]
    Network(CreateNetwork),
    /// Request to create a [`Container`]
    Container(CreateContainer),
}

impl FromEvent for CreateRequests {
    type Err = FromEventError;

    fn from_event(value: DeviceEvent) -> Result<Self, Self::Err> {
        match value.interface.as_str() {
            "io.edgehog.devicemanager.apps.CreateImageRequest" => {
                CreateImage::from_event(value).map(CreateRequests::Image)
            }
            "io.edgehog.devicemanager.apps.CreateVolumeRequest" => {
                CreateVolume::from_event(value).map(CreateRequests::Volume)
            }
            "io.edgehog.devicemanager.apps.CreateNetworkRequest" => {
                CreateNetwork::from_event(value).map(CreateRequests::Network)
            }
            _ => Err(FromEventError::Interface(value.interface.clone())),
        }
    }
}

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateImageRequest",
    path = "/image"
)]
pub struct CreateImage {
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) name: String,
    pub(crate) tag: String,
}

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateVolumeRequest",
    path = "/volume"
)]
pub struct CreateVolume {
    pub(crate) id: String,
    pub(crate) driver: String,
    pub(crate) options: Vec<String>,
}

impl TryFrom<CreateVolume> for Volume<String> {
    type Error = ReqError;

    fn try_from(value: CreateVolume) -> Result<Self, Self::Error> {
        let options = value
            .options
            .into_iter()
            .map(|option| {
                option
                    .split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .ok_or(ReqError::Option(option))
            })
            .try_collect()?;

        Ok(Volume::with_options(value.id, value.driver, options))
    }
}

/// Request to create a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateNetworkRequest",
    path = "/network",
    rename_all = "camelCase"
)]
pub struct CreateNetwork {
    pub(crate) id: String,
    pub(crate) driver: String,
    pub(crate) check_duplicate: bool,
    pub(crate) internal: bool,
    pub(crate) enable_ipv6: bool,
}

impl From<CreateNetwork> for Network<String> {
    fn from(value: CreateNetwork) -> Self {
        Network {
            id: None,
            name: value.id,
            driver: value.driver,
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
        }
    }
}

/// Request to create a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateContainerRequest",
    path = "/container",
    rename_all = "camelCase"
)]
pub struct CreateContainer {
    pub(crate) id: String,
    pub(crate) hostname: String,
    pub(crate) image_id: String,
    pub(crate) network_ids: Vec<String>,
    pub(crate) volume_ids: Vec<String>,
    pub(crate) restart_policy: String,
    pub(crate) env: Vec<String>,
    pub(crate) binds: Vec<String>,
    pub(crate) networks: Vec<String>,
    pub(crate) port_bindings: Vec<String>,
    pub(crate) privileged: bool,
}

impl TryFrom<CreateContainer> for ContainerNode {
    type Error = ReqError;

    fn try_from(value: CreateContainer) -> Result<Self, Self::Error> {
        let restart_policy = value
            .restart_policy
            .parse()
            .map_err(ReqError::RestartPolicy)?;
        let port_bindings = map_port_bindings(&value.port_bindings)?;

        let container = Container {
            id: None,
            name: value.id,
            image: value.image_id,
            network_ids: value.network_ids,
            hostname: Some(value.hostname),
            restart_policy: Some(restart_policy),
            env: value.env,
            binds: value.binds,
            port_bindings,
            privileged: value.privileged,
        };

        let volume_ids = value.volume_ids.into_iter().map(Id::new).collect();

        Ok(ContainerNode::new(container, volume_ids))
    }
}

/// Error from parsing a binding
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum BindingError {
    /// couldn't parse {binding} port {value}
    Port {
        /// Binding received
        binding: &'static str,
        /// Port of the binding
        value: String,
        /// Error converting the port
        #[source]
        source: ParseIntError,
    },
}

impl BindingError {
    fn port(binding: &'static str, value: String, source: ParseIntError) -> Self {
        Self::Port {
            binding,
            value,
            source,
        }
    }
}

/// Parses a binding in the form
///
/// ```plaintext
/// [ip:[hostPort:]]containerPort[/protocol]
/// ```
#[instrument]
pub(crate) fn parse_port_binding(input: &str) -> Result<ParsedBind, BindingError> {
    let (host_ip, host_port, rest) = parse_host_ip_port(input)?;

    let (container_port, protocol) = if let Some((port, proto)) = rest.split_once('/') {
        trace!("container port {port} and protocol {proto}");

        (port, Some(proto))
    } else {
        trace!("container port {rest}");

        (rest, None)
    };

    let container_port = container_port
        .parse()
        .map_err(|err| BindingError::port("container", container_port.to_string(), err))?;

    Ok(ParsedBind::new(
        container_port,
        protocol,
        host_ip,
        host_port,
    ))
}

#[instrument]
fn parse_host_ip_port(input: &str) -> Result<(Option<&str>, Option<u16>, &str), BindingError> {
    let Some((ip_or_port, rest)) = input.split_once(':') else {
        trace!("missing host ip or port: {input}");

        return Ok((None, None, input));
    };

    match rest.split_once(':') {
        Some((port, rest)) => {
            let port: u16 = port.parse().map_err(|err| BindingError::Port {
                binding: "host",
                value: port.to_string(),
                source: err,
            })?;

            trace!("found ip {ip_or_port} and port {port}");

            Ok((Some(ip_or_port), Some(port), rest))
        }
        None => {
            // Try to parse the ip as port
            if let Ok(port) = ip_or_port.parse::<u16>() {
                trace!("found port {port}");

                Ok((None, Some(port), rest))
            } else {
                trace!("found ip {ip_or_port}");
                Ok((Some(ip_or_port), None, rest))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedBind<'a> {
    port: u16,
    proto: Option<&'a str>,
    pub(crate) host: Binding<&'a str>,
}

impl<'a> ParsedBind<'a> {
    fn new(
        port: u16,
        proto: Option<&'a str>,
        host_ip: Option<&'a str>,
        host_port: Option<u16>,
    ) -> Self {
        Self {
            port,
            proto,
            host: Binding { host_ip, host_port },
        }
    }

    pub(crate) fn id(&self) -> String {
        let proto = self.proto.unwrap_or("tcp");

        format!("{}/{}", self.port, proto)
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{types::AstarteType, Value};

    use super::*;

    #[test]
    fn create_image_request() {
        let fields = [
            ("id", "id"),
            ("repo", "repo"),
            ("name", "name"),
            ("tag", "tag"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into()))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateImageRequest".to_string(),
            path: "/image".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateImage::from_event(event);

        let expect = CreateImage {
            id: "id".to_string(),
            repo: "repo".to_string(),
            name: "name".to_string(),
            tag: "tag".to_string(),
        };

        assert_eq!(create_image.unwrap(), expect);
    }

    #[test]
    fn create_volume() {
        let fields = [
            ("id", AstarteType::String("id".to_string())),
            ("driver", AstarteType::String("local".to_string())),
            (
                "options",
                AstarteType::StringArray(vec!["foo=bar".to_string()]),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateVolumeRequest".to_string(),
            path: "/volume".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateVolume::from_event(event);

        let expect = CreateVolume {
            id: "id".to_string(),
            driver: "local".to_string(),
            options: vec!["foo=bar".to_string()],
        };

        assert_eq!(create_image.unwrap(), expect);
    }

    #[test]
    fn create_network() {
        let fields = [
            ("id", AstarteType::String("id".to_string())),
            ("driver", AstarteType::String("bridged".to_string())),
            ("internal", AstarteType::Boolean(false)),
            ("checkDuplicate", AstarteType::Boolean(false)),
            ("enableIpv6", AstarteType::Boolean(true)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateNetworkRequest".to_string(),
            path: "/network".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateNetwork::from_event(event).unwrap();

        let expect = CreateNetwork {
            id: "id".to_string(),
            driver: "bridged".to_string(),
            internal: false,
            check_duplicate: false,
            enable_ipv6: true,
        };

        assert_eq!(create_image, expect);
    }

    #[test]
    fn create_container() {
        let fields = [
            ("id", AstarteType::String("id".to_string())),
            ("hostname", AstarteType::String("hostname".to_string())),
            ("imageId", AstarteType::String("imageId".to_string())),
            (
                "networkIds",
                AstarteType::StringArray(vec!["networkIds".to_string()]),
            ),
            (
                "volumeIds",
                AstarteType::StringArray(vec!["volumeIds".to_string()]),
            ),
            (
                "restartPolicy",
                AstarteType::String("restartPolicy".to_string()),
            ),
            ("env", AstarteType::StringArray(vec!["env".to_string()])),
            ("binds", AstarteType::StringArray(vec!["binds".to_string()])),
            (
                "networks",
                AstarteType::StringArray(vec!["networks".to_string()]),
            ),
            (
                "portBindings",
                AstarteType::StringArray(vec!["portBindings".to_string()]),
            ),
            ("privileged", AstarteType::Boolean(true)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateContainerRequest".to_string(),
            path: "/container".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateContainer::from_event(event).unwrap();

        let expect = CreateContainer {
            id: "id".to_string(),
            hostname: "hostname".to_string(),
            image_id: "imageId".to_string(),
            network_ids: vec!["networkIds".to_string()],
            volume_ids: vec!["volumeIds".to_string()],
            restart_policy: "restartPolicy".to_string(),
            env: vec!["env".to_string()],
            binds: vec!["binds".to_string()],
            networks: vec!["networks".to_string()],
            port_bindings: vec!["portBindings".to_string()],
            privileged: true,
        };

        assert_eq!(create_image, expect);
    }

    #[test]
    fn should_parse_port_binding() {
        let cases = [
            // ip:[hostPort:]containerPort[/protocol]
            (
                "1.1.1.1:80:90/udp",
                ParsedBind::new(90, Some("udp"), Some("1.1.1.1"), Some(80)),
            ),
            (
                "1.1.1.1:90/udp",
                ParsedBind::new(90, Some("udp"), Some("1.1.1.1"), None),
            ),
            (
                "1.1.1.1:90",
                ParsedBind::new(90, None, Some("1.1.1.1"), None),
            ),
            // [hostPort:]containerPort[/protocol]
            (
                "80:90/udp",
                ParsedBind::new(90, Some("udp"), None, Some(80)),
            ),
            ("90/udp", ParsedBind::new(90, Some("udp"), None, None)),
            ("90", ParsedBind::new(90, None, None, None)),
        ];

        for (case, expected) in cases {
            let parsed = parse_port_binding(case).unwrap();

            assert_eq!(parsed, expected, "failed to parse {case}");
        }
    }
}
