// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

//! Create image request

use std::str::FromStr;

use astarte_device_sdk::FromEvent;
use bollard::models::RestartPolicyNameEnum;
use tracing::{instrument, trace};

use crate::{container::Binding, requests::BindingError};

use super::{OptString, ReqUuid, VecReqUuid};

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
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateContainer {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) image_id: ReqUuid,
    pub(crate) network_ids: Option<VecReqUuid>,
    pub(crate) volume_ids: Option<VecReqUuid>,
    pub(crate) device_mapping_ids: Option<VecReqUuid>,
    pub(crate) device_request_ids: Option<VecReqUuid>,
    pub(crate) hostname: Option<String>,
    pub(crate) restart_policy: Option<String>,
    pub(crate) env: Option<Vec<String>>,
    pub(crate) binds: Option<Vec<String>>,
    pub(crate) network_mode: Option<String>,
    pub(crate) port_bindings: Option<Vec<String>>,
    pub(crate) extra_hosts: Option<Vec<String>>,
    pub(crate) cap_add: Option<Vec<String>>,
    pub(crate) cap_drop: Option<Vec<String>>,
    pub(crate) cpu_period: Option<i64>,
    pub(crate) cpu_quota: Option<i64>,
    pub(crate) cpu_realtime_period: Option<i64>,
    pub(crate) cpu_realtime_runtime: Option<i64>,
    pub(crate) memory: Option<i64>,
    pub(crate) memory_reservation: Option<i64>,
    pub(crate) memory_swap: Option<i64>,
    pub(crate) memory_swappiness: Option<i32>,
    pub(crate) volume_driver: Option<OptString>,
    pub(crate) storage_opt: Option<Vec<String>>,
    pub(crate) read_only_rootfs: Option<bool>,
    pub(crate) tmpfs: Option<Vec<String>>,
    pub(crate) privileged: Option<bool>,
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

    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub fn create_container_request_event<S: Display>(
        id: impl Display,
        deployment_id: impl Display,
        image_id: impl Display,
        image: &str,
        network_ids: &[S],
        device_mapping_ids: &[impl Display],
        device_request_ids: &[impl Display],
    ) -> DeviceEvent {
        let fields = [
            ("id", AstarteData::String(id.to_string())),
            (
                "deploymentId",
                AstarteData::String(deployment_id.to_string()),
            ),
            ("imageId", AstarteData::String(image_id.to_string())),
            ("volumeIds", AstarteData::StringArray(vec![])),
            (
                "deviceMappingIds",
                AstarteData::StringArray(
                    device_mapping_ids.iter().map(|d| d.to_string()).collect(),
                ),
            ),
            (
                "deviceRequestIds",
                AstarteData::StringArray(
                    device_request_ids.iter().map(|d| d.to_string()).collect(),
                ),
            ),
            ("image", AstarteData::String(image.to_string())),
            ("hostname", AstarteData::String("hostname".to_string())),
            ("restartPolicy", AstarteData::String("no".to_string())),
            ("env", AstarteData::StringArray(vec!["env".to_string()])),
            ("binds", AstarteData::StringArray(vec!["binds".to_string()])),
            ("networkMode", AstarteData::String("bridge".to_string())),
            (
                "networkIds",
                AstarteData::StringArray(network_ids.iter().map(|s| s.to_string()).collect()),
            ),
            (
                "portBindings",
                AstarteData::StringArray(vec!["80:80".to_string()]),
            ),
            (
                "extraHosts",
                AstarteData::StringArray(vec!["host.docker.internal:host-gateway".to_string()]),
            ),
            (
                "capAdd",
                AstarteData::StringArray(vec!["CAP_CHOWN".to_string()]),
            ),
            (
                "capDrop",
                AstarteData::StringArray(vec!["CAP_KILL".to_string()]),
            ),
            ("privileged", AstarteData::Boolean(false)),
            ("cpuPeriod", AstarteData::LongInteger(1000)),
            ("cpuQuota", AstarteData::LongInteger(100)),
            ("cpuRealtimePeriod", AstarteData::LongInteger(1000)),
            ("cpuRealtimeRuntime", AstarteData::LongInteger(100)),
            ("memory", AstarteData::LongInteger(4096)),
            ("memoryReservation", AstarteData::LongInteger(1024)),
            ("memorySwap", AstarteData::LongInteger(8192)),
            ("memorySwappiness", AstarteData::Integer(50)),
            ("volumeDriver", AstarteData::from("local")),
            (
                "storageOpt",
                AstarteData::from(vec!["size=1024k".to_string()]),
            ),
            ("readOnlyRootfs", AstarteData::from(true)),
            (
                "tmpfs",
                AstarteData::from(vec!["/run=rw,noexec,nosuid,size=65536k".to_string()]),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateContainerRequest".to_string(),
            path: "/container".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn create_container_request() {
        let id = ReqUuid(Uuid::new_v4());
        let deployment_id = ReqUuid(Uuid::new_v4());
        let image_id = ReqUuid(Uuid::new_v4());
        let network_ids = VecReqUuid(vec![ReqUuid(Uuid::new_v4())]);
        let device_mapping_ids = VecReqUuid(vec![ReqUuid(Uuid::new_v4())]);
        let device_request_ids = VecReqUuid(vec![ReqUuid(Uuid::new_v4())]);
        let event = create_container_request_event(
            id,
            deployment_id,
            image_id,
            "image",
            &network_ids,
            &device_mapping_ids,
            &device_request_ids,
        );

        let request = CreateContainer::from_event(event).unwrap();

        let expect = CreateContainer {
            id,
            deployment_id,
            image_id,
            network_ids: Some(network_ids),
            volume_ids: Some(VecReqUuid(vec![])),
            device_mapping_ids: Some(device_mapping_ids),
            device_request_ids: Some(device_request_ids),
            hostname: Some("hostname".to_string()),
            restart_policy: Some("no".to_string()),
            env: Some(vec!["env".to_string()]),
            binds: Some(vec!["binds".to_string()]),
            network_mode: Some("bridge".to_string()),
            port_bindings: Some(vec!["80:80".to_string()]),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
            cap_add: Some(vec!["CAP_CHOWN".to_string()]),
            cap_drop: Some(vec!["CAP_KILL".to_string()]),
            cpu_period: Some(1000),
            cpu_quota: Some(100),
            cpu_realtime_period: Some(1000),
            cpu_realtime_runtime: Some(100),
            memory: Some(4096),
            memory_reservation: Some(1024),
            memory_swap: Some(8192),
            memory_swappiness: Some(50),
            volume_driver: Some("local".to_string().into()),
            storage_opt: Some(vec!["size=1024k".to_string()]),
            read_only_rootfs: Some(true),
            tmpfs: Some(vec!["/run=rw,noexec,nosuid,size=65536k".to_string()]),
            privileged: Some(false),
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
