// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Create image request

use std::str::FromStr;

use astarte_device_sdk::FromEvent;
use tracing::{instrument, trace};

use crate::{
    container::{Binding, Container, PortBindingMap},
    requests::BindingError,
    service::{Id, ResourceType, ServiceError},
    store::container::RestartPolicy,
};

use super::ReqError;

/// Request to pull a Docker Container.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateContainerRequest",
    path = "/container",
    rename_all = "camelCase"
)]
pub struct CreateContainer {
    pub(crate) id: String,
    pub(crate) image_id: String,
    pub(crate) network_ids: Vec<String>,
    pub(crate) volume_ids: Vec<String>,
    // TODO: remove this image, use the image id
    pub(crate) image: String,
    pub(crate) hostname: String,
    pub(crate) restart_policy: String,
    pub(crate) env: Vec<String>,
    pub(crate) binds: Vec<String>,
    pub(crate) network_mode: String,
    pub(crate) port_bindings: Vec<String>,
    pub(crate) privileged: bool,
}

impl CreateContainer {
    pub(crate) fn dependencies(&self) -> Result<Vec<Id>, ServiceError> {
        std::iter::once(Id::try_from_str(ResourceType::Image, &self.image_id))
            .chain(
                self.network_ids
                    .iter()
                    .map(|id| Id::try_from_str(ResourceType::Network, id)),
            )
            .chain(
                self.volume_ids
                    .iter()
                    .map(|id| Id::try_from_str(ResourceType::Volume, id)),
            )
            .collect()
    }
}

impl TryFrom<CreateContainer> for Container<String> {
    type Error = ReqError;

    fn try_from(value: CreateContainer) -> Result<Self, Self::Error> {
        let hostname = if value.hostname.is_empty() {
            None
        } else {
            Some(value.hostname)
        };

        let restart_policy = RestartPolicy::from_str(&value.restart_policy)?.into();
        let port_bindings = PortBindingMap::try_from(value.port_bindings)?;

        Ok(Container {
            id: None,
            name: value.id,
            image: value.image,
            hostname,
            restart_policy,
            env: value.env,
            network_mode: value.network_mode,
            // Use the network ids
            networks: value.network_ids,
            binds: value.binds,
            port_bindings,
            privileged: value.privileged,
        })
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

/// Parses a binding in the form
///
/// ```plaintext
/// [ip:[hostPort:]]containerPort[/protocol]
/// ```
#[instrument]
pub(crate) fn parse_port_binding(input: &str) -> Result<ParsedBind, BindingError> {
    let (host_ip, host_port, rest) = parse_host_ip_port(input)?;

    let (container_port, protocol) = rest.split_once('/').map_or_else(
        || {
            trace!("container port {rest}");

            (rest, None)
        },
        |(port, proto)| {
            trace!("container port {port} and protocol {proto}");

            (port, Some(proto))
        },
    );

    let container_port = container_port.parse().map_err(|err| BindingError::Port {
        binding: "container",
        value: container_port.to_string(),
        source: err,
    })?;

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
        trace!("missing host ip or port, returning rest: {input}");

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

#[cfg(test)]
pub(crate) mod tests {

    use std::{collections::HashMap, fmt::Display};

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};
    use bollard::secret::RestartPolicyNameEnum;
    use pretty_assertions::assert_eq;

    use super::*;

    pub fn create_container_request_event(
        id: impl Display,
        image_id: &str,
        image: &str,
        network_ids: &[&str],
    ) -> DeviceEvent {
        let fields = [
            ("id", AstarteType::String(id.to_string())),
            ("imageId", AstarteType::String(image_id.to_string())),
            ("volumeIds", AstarteType::StringArray(vec![])),
            ("image", AstarteType::String(image.to_string())),
            ("hostname", AstarteType::String("hostname".to_string())),
            ("restartPolicy", AstarteType::String("no".to_string())),
            ("env", AstarteType::StringArray(vec!["env".to_string()])),
            ("binds", AstarteType::StringArray(vec!["binds".to_string()])),
            ("networkMode", AstarteType::String("bridge".to_string())),
            (
                "networkIds",
                AstarteType::StringArray(network_ids.iter().map(|s| s.to_string()).collect()),
            ),
            (
                "portBindings",
                AstarteType::StringArray(vec!["80:80".to_string()]),
            ),
            ("privileged", AstarteType::Boolean(false)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateContainerRequest".to_string(),
            path: "/container".to_string(),
            data: Value::Object(fields),
        }
    }

    #[test]
    fn create_container_request() {
        let event = create_container_request_event(
            "id",
            "image_id",
            "image",
            &["9808bbd5-2e81-4f99-83e7-7cc60623a196"],
        );

        let request = CreateContainer::from_event(event).unwrap();

        let expect = CreateContainer {
            id: "id".to_string(),
            image_id: "image_id".to_string(),
            network_ids: vec!["9808bbd5-2e81-4f99-83e7-7cc60623a196".to_string()],
            volume_ids: vec![],
            image: "image".to_string(),
            hostname: "hostname".to_string(),
            restart_policy: "no".to_string(),
            env: vec!["env".to_string()],
            binds: vec!["binds".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["80:80".to_string()],
            privileged: false,
        };

        assert_eq!(request, expect);

        let container = Container::try_from(request).unwrap();

        let exp = Container {
            id: None,
            name: "id",
            image: "image",
            network_mode: "bridge",
            networks: vec!["9808bbd5-2e81-4f99-83e7-7cc60623a196"],
            hostname: Some("hostname"),
            restart_policy: RestartPolicyNameEnum::NO,
            env: vec!["env"],
            binds: vec!["binds"],
            port_bindings: PortBindingMap::<&str>(HashMap::from_iter([(
                "80/tcp".to_string(),
                vec![Binding {
                    host_ip: None,
                    host_port: Some(80),
                }],
            )])),
            privileged: false,
        };

        assert_eq!(container, exp);
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
