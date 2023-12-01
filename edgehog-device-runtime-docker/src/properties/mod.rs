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

//! Property to send to Astarte.

use std::collections::{hash_map::Entry, HashMap};

use astarte_device_sdk::{
    error::Error as AstarteError,
    properties::PropAccess,
    store::{PropertyStore, StoredProp},
    types::{AstarteType, TypeError},
    Client, DeviceClient,
};
use async_trait::async_trait;
use petgraph::stable_graph::NodeIndex;

use crate::{
    request::BindingError,
    service::{Id, Node, NodeType, Nodes, ServiceError, State},
};

pub(crate) mod container;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

macro_rules! astarte_type {
    ($value:expr, $prop:ident, $field:ident => $typ:ty) => {{
        let $field: $typ = $value.try_into().map_err(|err| PropError::Type {
            field: stringify!($field),
            exp: stringify!($typ),
            backtrace: err,
        })?;

        $prop.$field.replace($field);
    }};
}

pub(crate) use astarte_type;

/// Error from handling the Astarte properties.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum PropError {
    /// endpoint missing prefix
    MissingPrefix,
    /// endpoint missing the id
    MissingId,
    /// couldn't convert field {field} into {exp}
    Type {
        field: &'static str,
        exp: &'static str,
        #[source]
        backtrace: TypeError,
    },
    /// couldn't convert property into {into}, since it's missing the field {field}
    MissingField {
        field: &'static str,
        into: &'static str,
    },
    /// couldn't convert property into {into}, unrecognized field {field}
    InvalidField { field: String, into: &'static str },
    /// couldn't parse option, expected key=value but got {0}
    Option(String),
    /// couldn't parse port binding
    Binding(#[from] BindingError),
}

impl PropError {
    const fn field(field: &'static str, into: &'static str) -> Self {
        PropError::MissingField { field, into }
    }
}

#[async_trait]
pub(crate) trait LoadProp:
    AvailableProp + TryFrom<StoredProp, Error = PropError> + TryInto<Self::Resource, Error = PropError>
{
    type Resource: Into<NodeType>;

    async fn load_resource<S>(
        device: &DeviceClient<S>,
        nodes: &mut Nodes,
    ) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let av_ifa_props = device.interface_props(Self::interface()).await?;

        let av_props = Self::from_props(av_ifa_props)?;

        av_props
            .into_iter()
            .try_for_each(|(id, av_prop)| -> Result<(), ServiceError> {
                let deps = av_prop.dependencies(nodes)?;

                let res: Self::Resource = av_prop.try_into().map_err(|err| ServiceError::Prop {
                    interface: Self::interface(),
                    backtrace: err,
                })?;

                let id = Id::new(id);

                nodes.add_node_sync(
                    id,
                    |id, idx| Ok(Node::with_state(id, idx, State::Stored(res.into()))),
                    &deps,
                )?;

                Ok(())
            })
    }

    fn dependencies(&self, nodes: &mut Nodes) -> Result<Vec<NodeIndex>, ServiceError>;

    fn from_props<I>(stored_props: I) -> Result<HashMap<String, Self>, ServiceError>
    where
        I: IntoIterator<Item = StoredProp>,
    {
        stored_props
            .into_iter()
            .map(Self::try_from)
            .try_fold(HashMap::<String, Self>::new(), |mut acc, item| {
                let item = item?;
                let id = item.id().to_string();

                match acc.entry(id) {
                    Entry::Occupied(mut entry) => {
                        entry.get_mut().merge(item);
                    }
                    Entry::Vacant(entry) => {
                        entry.insert(item);
                    }
                }

                Ok(acc)
            })
            .map_err(|err| ServiceError::Prop {
                interface: Self::interface(),
                backtrace: err,
            })
    }
}

pub(crate) fn replace_if_some<T>(value: &mut Option<T>, other: Option<T>) {
    if let Some(other) = other {
        value.replace(other);
    }
}

#[async_trait]
pub(crate) trait AvailableProp {
    fn interface() -> &'static str;

    fn id(&self) -> &str;

    async fn store<S>(&self, device: &DeviceClient<S>) -> Result<(), AstarteError>
    where
        S: PropertyStore;

    fn merge(&mut self, other: Self) -> &mut Self;

    fn parse_endpoint(endpoint: &str) -> Result<(&str, &str), PropError> {
        endpoint
            .strip_prefix('/')
            .ok_or(PropError::MissingPrefix)
            .and_then(|endpoint| endpoint.split_once('/').ok_or(PropError::MissingId))
    }

    fn parse_kv(key_value: &str) -> Option<(&str, &str)> {
        key_value.split_once('=')
    }

    async fn send<S, D>(
        &self,
        device: &DeviceClient<S>,
        field: &str,
        data: Option<D>,
    ) -> Result<(), AstarteError>
    where
        S: PropertyStore,
        D: Into<AstarteType> + Send,
    {
        let Some(data) = data else {
            return Ok(());
        };

        let endpoint = format!("/{}/{}", self.id(), field);

        device.send(Self::interface(), &endpoint, data).await
    }
}

#[cfg(test)]
mod tests {
    use crate::properties::image::AvailableImage;

    use super::*;

    #[test]
    fn should_extract_id() {
        let endpoint = "/id/name";

        let (id, field) = AvailableImage::<&str>::parse_endpoint(endpoint).unwrap();

        assert_eq!(id, "id");
        assert_eq!(field, "name");
    }
}
