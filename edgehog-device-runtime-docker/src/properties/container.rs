// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Property for the `AvailableContainers` interface.

use std::{collections::HashMap, hash::Hash};

use astarte_device_sdk::{
    error::Error as AstarteError,
    store::{PropertyStore, StoredProp},
    DeviceClient,
};
use async_trait::async_trait;
use itertools::Itertools;
use tracing::warn;

use crate::{
    container::{Container, PortBindingMap},
    request::{parse_port_binding, BindingError},
    service::{ContainerNode, Id, Nodes, ServiceError},
};

use super::{astarte_type, replace_if_some, AvailableProp, LoadProp, PropError};

#[derive(Debug, Clone, Default)]
pub(crate) struct AvailableContainer<S> {
    pub(crate) id: S,
    pub(crate) container_id: Option<S>,
    pub(crate) hostname: Option<S>,
    pub(crate) image_id: Option<S>,
    pub(crate) network_ids: Option<Vec<S>>,
    pub(crate) volume_ids: Option<Vec<S>>,
    pub(crate) restart_policy: Option<S>,
    pub(crate) env: Option<Vec<S>>,
    pub(crate) binds: Option<Vec<S>>,
    pub(crate) port_bindings: Option<PortBindingMap<S>>,
    pub(crate) privileged: Option<bool>,
}

impl<S> Eq for AvailableContainer<S> where S: Eq + Hash {}

impl<S> PartialEq for AvailableContainer<S>
where
    S: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
            && self.container_id.eq(&other.container_id)
            && self.hostname.eq(&other.hostname)
            && self.image_id.eq(&other.image_id)
            && self.network_ids.eq(&other.network_ids)
            && self.volume_ids.eq(&other.volume_ids)
            && self.restart_policy.eq(&other.restart_policy)
            && self.env.eq(&other.env)
            && self.binds.eq(&other.binds)
            && self.port_bindings.eq(&other.port_bindings)
            && self.privileged.eq(&other.privileged)
    }
}

impl<S> AvailableContainer<S> {
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableContainers";

    pub(crate) fn new(id: S) -> Self
    where
        S: Default,
    {
        Self {
            id,
            ..Default::default()
        }
    }
}
impl<'a> AvailableContainer<&'a str> {
    pub(crate) fn with_container(
        id: &'a str,
        container: &'a Container<String>,
        volume_ids: &'a [Id],
    ) -> Self {
        Self {
            id,
            container_id: container.id.as_deref(),
            hostname: container.hostname.as_deref(),
            image_id: container.hostname.as_deref(),
            network_ids: Some(container.network_ids.iter().map(|s| s.as_ref()).collect()),
            volume_ids: Some(volume_ids.iter().map(|s| s.as_str()).collect()),
            restart_policy: container.restart_policy.as_ref().map(|v| v.as_ref()),
            env: Some(container.env.iter().map(|s| s.as_ref()).collect()),
            binds: Some(container.binds.iter().map(|s| s.as_ref()).collect()),
            port_bindings: Some(PortBindingMap::from(&container.port_bindings)),
            privileged: Some(container.privileged),
        }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableContainer<S>
where
    S: AsRef<str> + Eq + Hash + Sync,
{
    fn interface() -> &'static str {
        Self::INTERFACE
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }

    async fn store<T>(&self, device: &DeviceClient<T>) -> Result<(), AstarteError>
    where
        T: PropertyStore,
    {
        let container_id = self.container_id.as_ref().map(|s| s.as_ref());
        let hostname = self.hostname.as_ref().map(|s| s.as_ref());
        let image_id = self.image_id.as_ref().map(|s| s.as_ref());
        let network_ids = self
            .network_ids
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_ref().to_string()).collect_vec());
        let volume_ids = self
            .volume_ids
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_ref().to_string()).collect_vec());
        let restart_policy = self.restart_policy.as_ref().map(|s| s.as_ref());
        let env = self
            .env
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_ref().to_string()).collect_vec());
        let binds = self
            .binds
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_ref().to_string()).collect_vec());
        let port_bindings: Option<Vec<String>> = self.port_bindings.as_ref().map(|v| v.into());

        self.send(device, "id", container_id).await?;
        self.send(device, "hostname", hostname).await?;
        self.send(device, "imageId", image_id).await?;
        self.send(device, "networkIds", network_ids).await?;
        self.send(device, "volumeIds", volume_ids).await?;
        self.send(device, "restart_policy", restart_policy).await?;
        self.send(device, "env", env).await?;
        self.send(device, "binds", binds).await?;
        self.send(device, "port_bindings", port_bindings).await?;
        self.send(device, "privileged", self.privileged).await?;

        Ok(())
    }

    fn merge(&mut self, other: Self) -> &mut Self {
        self.id = other.id;
        replace_if_some(&mut self.container_id, other.container_id);
        replace_if_some(&mut self.hostname, other.hostname);
        replace_if_some(&mut self.image_id, other.image_id);
        replace_if_some(&mut self.network_ids, other.network_ids);
        replace_if_some(&mut self.volume_ids, other.volume_ids);
        replace_if_some(&mut self.restart_policy, other.restart_policy);
        replace_if_some(&mut self.env, other.env);
        replace_if_some(&mut self.binds, other.binds);
        replace_if_some(&mut self.port_bindings, other.port_bindings);
        replace_if_some(&mut self.privileged, other.privileged);

        self
    }
}

impl LoadProp for AvailableContainer<String> {
    type Resource = ContainerNode;

    fn dependencies(
        &self,
        nodes: &mut Nodes,
    ) -> Result<Vec<petgraph::prelude::NodeIndex>, ServiceError> {
        self.volume_ids
            .as_ref()
            .map(|ids| {
                ids.iter()
                    .map(|id| {
                        nodes
                            .get_idx(id)
                            .ok_or_else(|| ServiceError::MissingNode(id.to_string()))
                    })
                    .collect()
            })
            .transpose()
            .map(Option::unwrap_or_default)
    }
}

impl TryFrom<StoredProp> for AvailableContainer<String> {
    type Error = PropError;

    fn try_from(value: StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_container = AvailableContainer::new(id.to_string());
        let prop = value.value;

        match field {
            "containerId" => astarte_type!(prop, av_container, container_id => String),
            "hostname" => astarte_type!(prop, av_container, hostname => String),
            "imageId" => astarte_type!(prop, av_container, image_id => String),
            "networkIds" => astarte_type!(prop, av_container, network_ids => Vec<String>),
            "volumeIds" => astarte_type!(prop, av_container, volume_ids => Vec<String>),
            "restartPolicy" => astarte_type!(prop, av_container, restart_policy => String),
            "env" => astarte_type!(prop, av_container, env => Vec<String>),
            "binds" => astarte_type!(prop, av_container, binds => Vec<String>),
            "portBindings" => {
                let port_bindings: Vec<String> =
                    prop.try_into().map_err(|err| PropError::Type {
                        field: stringify!(port_bindings),
                        exp: stringify!(Vec<String>),
                        backtrace: err,
                    })?;

                let bindings = map_port_bindings(&port_bindings)?;

                av_container.port_bindings.replace(bindings);
            }
            "privileged" => astarte_type!(prop, av_container, privileged => bool),
            _ => {
                warn!(
                    "unrecognized field for stored property for interface {}/{}:{}",
                    value.interface, value.path, value.interface_major
                );
            }
        }

        Ok(av_container)
    }
}

impl TryFrom<AvailableContainer<String>> for ContainerNode {
    type Error = PropError;

    fn try_from(value: AvailableContainer<String>) -> Result<Self, Self::Error> {
        let image = value
            .image_id
            .ok_or(PropError::field("image", "Container"))?;
        let network_ids = value
            .network_ids
            .ok_or(PropError::field("network_ids", "Container"))?;
        let restart_policy = value
            .restart_policy
            .map(|restart| {
                restart.parse().map_err(|_| PropError::InvalidField {
                    field: "restart_policy".to_string(),
                    into: "Container",
                })
            })
            .transpose()?;
        let env = value.env.unwrap_or_default();
        let binds = value.binds.unwrap_or_default();
        let port_bindings = value
            .port_bindings
            .ok_or(PropError::field("port_bindings", "Container"))?;
        let privileged = value
            .privileged
            .ok_or(PropError::field("privileged", "Container"))?;

        let container = Container {
            id: value.container_id,
            name: value.id,
            image,
            network_ids,
            hostname: value.hostname,
            restart_policy,
            env,
            binds,
            port_bindings,
            privileged,
        };

        let volumes = value
            .volume_ids
            .ok_or(PropError::field("volume_ids", "Container"))?
            .into_iter()
            .map(Id::new)
            .collect();

        Ok(ContainerNode::new(container, volumes))
    }
}

fn map_port_bindings(value: &[String]) -> Result<PortBindingMap<String>, BindingError> {
    value
        .iter()
        .try_fold(HashMap::new(), |mut acc, s| -> Result<_, BindingError> {
            let bind = parse_port_binding(s)?;

            let port_binds = match acc.entry(bind.id()).or_default() {
                Some(entry) => entry,
                entry @ None => entry.insert(Vec::new()),
            };

            port_binds.push(bind.host.into());

            Ok(acc)
        })
        .map(PortBindingMap)
}
