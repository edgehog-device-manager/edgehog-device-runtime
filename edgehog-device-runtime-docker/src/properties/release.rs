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

use std::hash::Hash;

use astarte_device_sdk::{store::StoredProp, Client};
use async_trait::async_trait;
use itertools::Itertools;
use tracing::warn;

use crate::{
    properties::astarte_type,
    service::{Id, Release},
};

use super::{replace_if_some, AvailableProp, LoadProp, PropError};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) struct AvailableRelease<S> {
    pub(crate) id: S,
    pub(crate) application_id: Option<S>,
    pub(crate) started: Option<bool>,
    pub(crate) containers: Option<Vec<S>>,
}

impl<S> AvailableRelease<S> {
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableRelease";

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

impl<'a> AvailableRelease<&'a str> {
    pub(crate) fn with_release(id: &'a str, release: &'a Release) -> Self {
        Self {
            id,
            application_id: Some(&release.application_id),
            started: Some(release.started),
            containers: Some(release.containers.iter().map(Id::as_str).collect()),
        }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableRelease<S>
where
    S: AsRef<str> + Eq + Hash + Sync,
{
    fn interface() -> &'static str {
        Self::INTERFACE
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }

    async fn store<D>(&self, device: &D) -> Result<(), PropError>
    where
        D: Client + Sync,
    {
        let application_id = self.application_id.as_ref().map(AsRef::as_ref);
        let containers = self
            .containers
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_ref().to_string()).collect_vec());

        self.send(device, "application_id", application_id).await?;
        self.send(device, "started", self.started).await?;
        self.send(device, "containers", containers).await?;

        Ok(())
    }
}

impl LoadProp for AvailableRelease<String> {
    type Res = Release;

    fn merge(&mut self, other: Self) -> &mut Self {
        self.id = other.id;

        replace_if_some(&mut self.application_id, other.application_id);
        replace_if_some(&mut self.containers, other.containers);
        replace_if_some(&mut self.started, other.started);

        self
    }
}

impl TryFrom<StoredProp> for AvailableRelease<String> {
    type Error = PropError;

    fn try_from(value: StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_release = AvailableRelease::new(id.to_string());
        let prop = value.value;

        match field {
            "application_id" => astarte_type!(prop, av_release, application_id => String),
            "started" => astarte_type!(prop, av_release, started => bool),
            "containers" => astarte_type!(prop, av_release, containers => Vec<String>),
            _ => {
                warn!(
                    "unrecognized field for stored property for interface {}/{}:{}",
                    value.interface, value.path, value.interface_major
                );
            }
        }

        Ok(av_release)
    }
}

impl TryFrom<AvailableRelease<String>> for Release {
    type Error = PropError;

    fn try_from(value: AvailableRelease<String>) -> Result<Self, Self::Error> {
        let application_id = value
            .application_id
            .ok_or(PropError::field("application_id", "Release"))?;
        let started = value
            .started
            .ok_or(PropError::field("started", "Release"))?;
        let containers = value
            .containers
            .ok_or(PropError::field("containers", "Release"))?
            .into_iter()
            .map(Id::new)
            .collect_vec();

        Ok(Release {
            application_id,
            started,
            containers,
        })
    }
}
