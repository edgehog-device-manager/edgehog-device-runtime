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

//! Property for the `AvailableVolume` interface.

use std::{collections::HashMap, fmt::Display, hash::Hash};

use astarte_device_sdk::{
    error::Error as AstarteError,
    store::{PropertyStore, StoredProp},
    DeviceClient,
};
use async_trait::async_trait;
use itertools::Itertools;
use tracing::warn;

use crate::volume::Volume;

use super::{astarte_type, AvailableProp, LoadProp, PropError};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AvailableVolume<S>
where
    S: Hash + Eq,
{
    pub(crate) id: S,
    pub(crate) created: Option<bool>,
    pub(crate) driver: Option<S>,
    pub(crate) options: Option<HashMap<S, S>>,
}

impl<S> AvailableVolume<S>
where
    S: Hash + Eq,
{
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableImages";

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

impl<'a> AvailableVolume<&'a str> {
    pub(crate) fn with_volume(id: &'a str, volume: &'a Volume<String>) -> Self {
        Self::with_created(id, volume, false)
    }

    pub(crate) fn with_created(id: &'a str, volume: &'a Volume<String>, created: bool) -> Self {
        Self {
            id,
            created: Some(created),
            driver: Some(volume.driver.as_str()),
            options: Some(
                volume
                    .driver_opts
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect(),
            ),
        }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableVolume<S>
where
    S: AsRef<str> + Hash + Display + Eq + Sync + Send,
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
        let driver = self.driver.as_ref().map(S::as_ref);
        let options = self.driver.as_ref().map(S::as_ref);

        self.send(device, "created", self.created).await?;
        self.send(device, "driver", driver).await?;
        self.send(device, "options", options).await?;

        Ok(())
    }

    fn merge(self, other: Self) -> Self {
        Self {
            id: other.id,
            created: other.created.or(self.created),
            driver: other.driver.or(self.driver),
            options: other.options.or(self.options),
        }
    }
}

impl LoadProp for AvailableVolume<String> {
    type Resource = Volume<String>;
}

impl TryFrom<StoredProp> for AvailableVolume<String> {
    type Error = PropError;

    fn try_from(value: StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_vol = AvailableVolume::new(id.to_string());
        let prop = value.value;

        match field {
            "created" => astarte_type!(prop, av_vol, created => bool),
            "driver" => astarte_type!(prop, av_vol, driver => String),
            "options" => {
                let options: Vec<String> = prop.try_into().map_err(|err| PropError::Type {
                    field: "options",
                    exp: "StringArray",
                    backtrace: err,
                })?;

                let hm_opts = options
                    .into_iter()
                    .map(|opt| {
                        Self::parse_kv(&opt)
                            .map(|(k, v)| (k.to_string(), v.to_string()))
                            .ok_or_else(|| PropError::Option(opt.clone()))
                    })
                    .try_collect()?;

                av_vol.options.replace(hm_opts);
            }
            _ => warn!(
                "unrecognized field for stored property for interface {}/{}:{}",
                value.interface, value.path, value.interface_major
            ),
        }

        Ok(av_vol)
    }
}

impl TryFrom<AvailableVolume<String>> for Volume<String> {
    type Error = PropError;

    fn try_from(value: AvailableVolume<String>) -> Result<Self, Self::Error> {
        let name = value.id;
        let tag = value.driver.ok_or(PropError::field("driver", "Volume"))?;
        let options = value.options.ok_or(PropError::field("repo", "Volume"))?;

        Ok(Self::with_options(name, tag, options))
    }
}

impl From<AvailableVolume<&str>> for AvailableVolume<String> {
    fn from(value: AvailableVolume<&str>) -> Self {
        AvailableVolume {
            id: value.id.to_string(),
            created: value.created,
            driver: value.driver.map(ToString::to_string),
            options: value.options.map(|opts| {
                opts.iter()
                    .map(|(&k, &v)| (k.to_string(), v.to_string()))
                    .collect()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{interface::def::Ownership, types::AstarteType};

    use super::*;

    #[test]
    fn should_convert_volume() {
        let cases = [
            (
                "/1234/created",
                AstarteType::Boolean(true),
                AvailableVolume {
                    id: "1234",
                    created: Some(true),
                    ..Default::default()
                },
            ),
            (
                "/1234/driver",
                AstarteType::String("local".to_string()),
                AvailableVolume {
                    id: "1234",
                    driver: Some("local"),
                    ..Default::default()
                },
            ),
            (
                "/1234/options",
                AstarteType::StringArray(vec!["foo=bar".to_string()]),
                AvailableVolume {
                    id: "1234",
                    options: Some([("foo", "bar")].into_iter().collect()),
                    ..Default::default()
                },
            ),
        ];

        for (path, value, expected) in cases {
            let prop = StoredProp {
                interface: AvailableVolume::<&str>::INTERFACE.to_string(),
                path: path.to_string(),
                value,
                interface_major: 1,
                ownership: Ownership::Device,
            };

            let image = AvailableVolume::try_from(prop).unwrap();

            assert_eq!(image, expected.into());
        }
    }
}
