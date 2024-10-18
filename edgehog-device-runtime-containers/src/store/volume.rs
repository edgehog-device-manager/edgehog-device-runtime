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

//! Persists the state of an [`Image`](crate::docker::image::Image)

use std::{borrow::Cow, collections::HashMap};

use serde::{Deserialize, Serialize};

use crate::volume::Volume;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VolumeState<'a> {
    name: Cow<'a, str>,
    driver: Cow<'a, str>,
    driver_opts: HashMap<Cow<'a, str>, Cow<'a, str>>,
}

impl<'a, S> From<&'a Volume<S>> for VolumeState<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Volume<S>) -> Self {
        Self {
            name: Cow::Borrowed(value.name.as_ref()),
            driver: Cow::Borrowed(value.driver.as_ref()),
            driver_opts: value
                .driver_opts
                .iter()
                .map(|(k, v)| (Cow::Borrowed(k.as_str()), Cow::Borrowed(v.as_ref())))
                .collect(),
        }
    }
}

impl<'a> From<VolumeState<'a>> for Volume<String> {
    fn from(value: VolumeState<'a>) -> Self {
        Self {
            name: value.name.into(),
            driver: value.driver.into(),
            driver_opts: value
                .driver_opts
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_convert_volume() {
        let volume = Volume::new(
            "name",
            "driver",
            HashMap::from([("name".to_string(), "value")]),
        );

        let state = VolumeState::from(&volume);

        assert_eq!(state.name, volume.name);
        assert_eq!(state.driver, volume.driver);

        let hm: HashMap<String, &str> = state
            .driver_opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_ref()))
            .collect();

        assert_eq!(hm, volume.driver_opts);

        let from = Volume::from(state);

        assert_eq!(from, volume);
    }
}
