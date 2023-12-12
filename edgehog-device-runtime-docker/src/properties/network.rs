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

//! Property for the `AvailableNetworks` interface.

use astarte_device_sdk::{error::Error as AstarteError, store::StoredProp, Client};
use async_trait::async_trait;
use tracing::warn;

use crate::{
    docker::network::Network,
    service::{nodes::Nodes, ServiceError},
};

use super::{astarte_type, replace_if_some, AvailableProp, LoadProp, PropError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AvailableNetwork<S> {
    pub(crate) id: S,
    pub(crate) network_id: Option<S>,
    pub(crate) driver: Option<S>,
    pub(crate) check_duplicate: Option<bool>,
    pub(crate) internal: Option<bool>,
    pub(crate) enable_ipv6: Option<bool>,
}

impl<S> AvailableNetwork<S> {
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableNetworks";

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

impl<'a> AvailableNetwork<&'a str> {
    pub(crate) fn with_network(id: &'a str, network: &'a Network<String>) -> Self {
        Self {
            id,
            network_id: network.id.as_deref(),
            driver: Some(network.driver.as_str()),
            check_duplicate: Some(network.check_duplicate),
            internal: Some(network.internal),
            enable_ipv6: Some(network.enable_ipv6),
        }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableNetwork<S>
where
    S: AsRef<str> + Sync,
{
    fn interface() -> &'static str {
        Self::INTERFACE
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }

    async fn store<D>(&self, device: &D) -> Result<(), AstarteError>
    where
        D: Client + Sync,
    {
        let id = self.network_id.as_ref().map(S::as_ref);
        let driver = self.driver.as_ref().map(S::as_ref);

        self.send(device, "id", id).await?;
        self.send(device, "driver", driver).await?;
        self.send(device, "checkDuplicate", self.check_duplicate)
            .await?;
        self.send(device, "internal", self.internal).await?;
        self.send(device, "enableIpv6", self.enable_ipv6).await?;

        Ok(())
    }

    fn merge(&mut self, other: Self) -> &mut Self {
        self.id = other.id;
        replace_if_some(&mut self.network_id, other.network_id);
        replace_if_some(&mut self.driver, other.driver);
        replace_if_some(&mut self.check_duplicate, other.check_duplicate);
        replace_if_some(&mut self.internal, other.internal);
        replace_if_some(&mut self.enable_ipv6, other.enable_ipv6);

        self
    }
}

impl LoadProp for AvailableNetwork<String> {
    type Resource = Network<String>;

    fn dependencies(
        &self,
        _nodes: &mut Nodes,
    ) -> Result<Vec<petgraph::prelude::NodeIndex>, ServiceError> {
        Ok(Vec::new())
    }
}

impl TryFrom<StoredProp> for AvailableNetwork<String> {
    type Error = PropError;

    fn try_from(value: StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_net = AvailableNetwork::new(id.to_string());
        let prop = value.value;

        match field {
            "id" => astarte_type!(prop, av_net, network_id => String),
            "driver" => astarte_type!(prop, av_net, driver => String),
            "checkDuplicate" => astarte_type!(prop, av_net, check_duplicate => bool),
            "internal" => astarte_type!(prop, av_net, internal => bool),
            "enableIpv6" => astarte_type!(prop, av_net, enable_ipv6 => bool),
            _ => {
                warn!(
                    "unrecognized field for stored property for interface {}/{}:{}",
                    value.interface, value.path, value.interface_major
                );
            }
        }

        Ok(av_net)
    }
}

impl TryFrom<AvailableNetwork<String>> for Network<String> {
    type Error = PropError;

    fn try_from(value: AvailableNetwork<String>) -> Result<Self, Self::Error> {
        let driver = value.driver.ok_or(PropError::field("driver", "Network"))?;
        let check_duplicate = value
            .check_duplicate
            .ok_or(PropError::field("check_duplicate", "Network"))?;
        let internal = value
            .internal
            .ok_or(PropError::field("internal", "Network"))?;
        let enable_ipv6 = value
            .enable_ipv6
            .ok_or(PropError::field("enable_ipv6", "Network"))?;

        Ok(Network {
            id: value.network_id,
            name: value.id,
            driver,
            check_duplicate,
            internal,
            enable_ipv6,
        })
    }
}

impl From<AvailableNetwork<&str>> for AvailableNetwork<String> {
    fn from(value: AvailableNetwork<&str>) -> Self {
        AvailableNetwork {
            id: value.id.to_string(),
            network_id: value.network_id.map(ToString::to_string),
            driver: value.driver.map(ToString::to_string),
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
        }
    }
}
