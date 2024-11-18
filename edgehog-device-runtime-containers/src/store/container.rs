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

//! Persists the state of a [`Container`](crate::docker::container::Container)

use std::{borrow::Cow, collections::HashMap, hash::Hash, str::FromStr};

use bollard::secret::RestartPolicyNameEnum;
use serde::{Deserialize, Serialize};

use crate::container::{Binding, Container, PortBindingMap};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ContainerState<'a> {
    id: Option<Cow<'a, str>>,
    name: Cow<'a, str>,
    image: Cow<'a, str>,
    network_mode: Cow<'a, str>,
    networks: Vec<Cow<'a, str>>,
    hostname: Option<Cow<'a, str>>,
    restart_policy: RestartPolicy,
    env: Vec<Cow<'a, str>>,
    binds: Vec<Cow<'a, str>>,
    port_bindings: HashMap<Cow<'a, str>, Vec<PortBinding<'a>>>,
    privileged: bool,
}

impl<'a, S> From<&'a Container<S>> for ContainerState<'a>
where
    S: AsRef<str> + Eq + Hash,
{
    fn from(value: &'a Container<S>) -> Self {
        let networks = value
            .networks
            .iter()
            .map(|s| Cow::Borrowed(s.as_ref()))
            .collect();

        let env = value
            .env
            .iter()
            .map(|s| Cow::Borrowed(s.as_ref()))
            .collect();

        let binds = value
            .binds
            .iter()
            .map(|s| Cow::Borrowed(s.as_ref()))
            .collect();

        let port_bindings = value
            .port_bindings
            .iter()
            .map(|(k, v)| {
                let binds = v.iter().map(PortBinding::from).collect();

                (Cow::Borrowed(k.as_ref()), binds)
            })
            .collect();

        Self {
            id: value.id.as_deref().map(Cow::Borrowed),
            name: Cow::Borrowed(value.name.as_ref()),
            image: Cow::Borrowed(value.image.as_ref()),
            network_mode: Cow::Borrowed(value.network_mode.as_ref()),
            networks,
            hostname: value.hostname.as_ref().map(|s| Cow::Borrowed(s.as_ref())),
            restart_policy: value.restart_policy.into(),
            env,
            binds,
            port_bindings,
            privileged: value.privileged,
        }
    }
}

impl<'a> From<ContainerState<'a>> for Container<String> {
    fn from(value: ContainerState<'a>) -> Self {
        let port_bindings = value
            .port_bindings
            .into_iter()
            .map(|(k, v)| {
                let binds = v.into_iter().map(Binding::from).collect();

                (String::from(k), binds)
            })
            .collect();

        Self {
            id: value.id.map(Cow::into),
            name: value.name.into(),
            image: value.image.into(),
            network_mode: value.network_mode.into(),
            networks: value.networks.into_iter().map(Cow::into).collect(),
            hostname: value.hostname.map(Cow::into),
            restart_policy: value.restart_policy.into(),
            env: value.env.into_iter().map(Cow::into).collect(),
            binds: value.binds.into_iter().map(Cow::into).collect(),
            port_bindings: PortBindingMap(port_bindings),
            privileged: value.privileged,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PortBinding<'a> {
    /// Host IP
    pub host_ip: Option<Cow<'a, str>>,
    /// Host port
    pub host_port: Option<u16>,
}

impl<'a, S> From<&'a Binding<S>> for PortBinding<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Binding<S>) -> Self {
        Self {
            host_ip: value.host_ip.as_ref().map(|s| Cow::Borrowed(s.as_ref())),
            host_port: value.host_port,
        }
    }
}

impl From<PortBinding<'_>> for Binding<String> {
    fn from(value: PortBinding<'_>) -> Self {
        Binding {
            host_ip: value.host_ip.map(Cow::into),
            host_port: value.host_port,
        }
    }
}

/// couldn't parse restart policy {value}
#[derive(Debug, thiserror::Error, displaydoc::Display, PartialEq)]
pub struct RestartPolicyError {
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
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

impl From<RestartPolicyNameEnum> for RestartPolicy {
    fn from(value: RestartPolicyNameEnum) -> Self {
        match value {
            RestartPolicyNameEnum::EMPTY => RestartPolicy::Empty,
            RestartPolicyNameEnum::NO => RestartPolicy::No,
            RestartPolicyNameEnum::ALWAYS => RestartPolicy::Always,
            RestartPolicyNameEnum::UNLESS_STOPPED => RestartPolicy::UnlessStopped,
            RestartPolicyNameEnum::ON_FAILURE => RestartPolicy::OnFailure,
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
mod tests {
    use super::*;

    #[test]
    fn should_convert_to_state() {
        let container = Container {
            id: Some("id".to_string()),
            name: "name",
            image: "image",
            network_mode: "bridge",
            networks: vec!["network1", "network2"],
            hostname: Some("hostname"),
            restart_policy: RestartPolicyNameEnum::ON_FAILURE,
            env: vec!["env1=1", "env2="],
            binds: vec!["bind"],
            port_bindings: PortBindingMap(HashMap::from([(
                "bind".to_string(),
                vec![Binding {
                    host_ip: Some("ip"),
                    host_port: Some(9000),
                }],
            )])),
            privileged: false,
        };

        let state = ContainerState::from(&container);

        assert_eq!(state.id.as_ref().unwrap(), container.id.as_ref().unwrap());
        assert_eq!(state.name, container.name);
        assert_eq!(state.network_mode, container.network_mode);
        assert_eq!(state.networks, container.networks);
        assert_eq!(state.hostname.as_deref(), container.hostname);
        assert_eq!(state.restart_policy, RestartPolicy::OnFailure);
        assert_eq!(state.env, container.env);
        assert_eq!(state.binds, container.binds);
        assert_eq!(
            *state.port_bindings.get("bind").unwrap(),
            vec![PortBinding {
                host_ip: Some("ip".into()),
                host_port: Some(9000)
            }]
        );

        let back = Container::from(state);

        assert_eq!(back, container);
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
