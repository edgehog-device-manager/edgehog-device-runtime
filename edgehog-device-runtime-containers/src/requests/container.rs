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
use bollard::secret::RestartPolicyNameEnum;
use tracing::{instrument, trace};

use crate::{container::Binding, requests::BindingError};

use super::{ReqUuid, VecReqUuid};

/// couldn't parse restart policy {value}
#[derive(Debug, thiserror::Error, displaydoc::Display, PartialEq)]
pub struct RestartPolicyError {
    value: String,
}

/// Request to pull a Docker Container.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateContainerRequest",
    path = "/container",
    rename_all = "camelCase"
)]
pub struct CreateContainer {
    pub(crate) id: ReqUuid,
    pub(crate) deployment_id: ReqUuid,
    pub(crate) image_id: ReqUuid,
    pub(crate) network_ids: VecReqUuid,
    pub(crate) volume_ids: VecReqUuid,
    pub(crate) hostname: String,
    pub(crate) restart_policy: String,
    pub(crate) env: Vec<String>,
    pub(crate) binds: Vec<String>,
    pub(crate) network_mode: String,
    pub(crate) port_bindings: Vec<String>,
    pub(crate) privileged: bool,
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
pub(crate) fn parse_port_binding(input: &str) -> Result<ParsedBind<'_>, BindingError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestartPolicy {
    Empty,
    No,
    Always,
    UnlessStopped,
    OnFailure,
}

impl FromStr for RestartPolicy {
    type Err = RestartPolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "" => Ok(RestartPolicy::Empty),
            "no" => Ok(RestartPolicy::No),
            "always" => Ok(RestartPolicy::Always),
            "unless-stopped" => Ok(RestartPolicy::UnlessStopped),
            "on-failure" => Ok(RestartPolicy::OnFailure),
            _ => Err(RestartPolicyError {
                value: s.to_string(),
            }),
        }
    }
}

impl From<RestartPolicy> for RestartPolicyNameEnum {
    fn from(value: RestartPolicy) -> Self {
        match value {
            RestartPolicy::Empty => RestartPolicyNameEnum::EMPTY,
            RestartPolicy::No => RestartPolicyNameEnum::NO,
            RestartPolicy::Always => RestartPolicyNameEnum::ALWAYS,
            RestartPolicy::UnlessStopped => RestartPolicyNameEnum::UNLESS_STOPPED,
            RestartPolicy::OnFailure => RestartPolicyNameEnum::ON_FAILURE,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {

    use std::fmt::Display;

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub fn create_container_request_event<S: Display>(
        id: impl Display,
        deployment_id: impl Display,
        image_id: impl Display,
        image: &str,
        network_ids: &[S],
    ) -> DeviceEvent {
        let fields = [
            ("id", AstarteType::String(id.to_string())),
            (
                "deploymentId",
                AstarteType::String(deployment_id.to_string()),
            ),
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
        let id = ReqUuid(Uuid::new_v4());
        let deployment_id = ReqUuid(Uuid::new_v4());
        let image_id = ReqUuid(Uuid::new_v4());
        let network_ids = VecReqUuid(vec![ReqUuid(Uuid::new_v4())]);
        let event =
            create_container_request_event(id, deployment_id, image_id, "image", &network_ids);

        let request = CreateContainer::from_event(event).unwrap();

        let expect = CreateContainer {
            id,
            deployment_id,
            image_id,
            network_ids,
            volume_ids: VecReqUuid(vec![]),
            hostname: "hostname".to_string(),
            restart_policy: "no".to_string(),
            env: vec!["env".to_string()],
            binds: vec!["binds".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["80:80".to_string()],
            privileged: false,
        };

        assert_eq!(request, expect);
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

    #[test]
    fn parse_restart_policy() {
        let cases = [
            ("", RestartPolicy::Empty),
            ("no", RestartPolicy::No),
            ("unless-stopped", RestartPolicy::UnlessStopped),
            ("on-failure", RestartPolicy::OnFailure),
            ("on-failure", RestartPolicy::OnFailure),
        ];

        for (case, exp) in cases {
            let policy = RestartPolicy::from_str(case).unwrap();

            assert_eq!(policy, exp);
        }

        let err = RestartPolicy::from_str("bar").unwrap_err();
        assert_eq!(
            err,
            RestartPolicyError {
                value: "bar".to_string()
            }
        );

        let err = RestartPolicy::from_str("NO").unwrap_err();
        assert_eq!(
            err,
            RestartPolicyError {
                value: "NO".to_string()
            }
        );

        let err = RestartPolicy::from_str("on_failure").unwrap_err();
        assert_eq!(
            err,
            RestartPolicyError {
                value: "on_failure".to_string()
            }
        );
    }
}
