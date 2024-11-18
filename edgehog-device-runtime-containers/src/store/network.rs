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

//! Persists the state of an [`Network`](crate::docker::network::Network)

use std::{borrow::Cow, collections::HashMap};

use serde::{Deserialize, Serialize};

use crate::network::Network;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NetworkState<'a> {
    id: Option<Cow<'a, str>>,
    name: Cow<'a, str>,
    driver: Cow<'a, str>,
    check_duplicate: bool,
    internal: bool,
    enable_ipv6: bool,
    driver_opts: HashMap<Cow<'a, str>, Cow<'a, str>>,
}

impl<'a, S> From<&'a Network<S>> for NetworkState<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Network<S>) -> Self {
        Self {
            id: value.id.as_deref().map(Cow::Borrowed),
            name: Cow::Borrowed(value.name.as_ref()),
            driver: Cow::Borrowed(value.driver.as_ref()),
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
            driver_opts: value
                .driver_opts
                .iter()
                .map(|(k, v)| (Cow::Borrowed(k.as_str()), Cow::Borrowed(v.as_ref())))
                .collect(),
        }
    }
}

impl<'a> From<NetworkState<'a>> for Network<String> {
    fn from(value: NetworkState<'a>) -> Self {
        Self {
            id: value.id.map(Cow::into),
            name: value.name.into(),
            driver: value.driver.into(),
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
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
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_convert_to_state() {
        let net = Network {
            id: Some("id".to_string()),
            name: "name",
            driver: "driver",
            check_duplicate: true,
            internal: false,
            enable_ipv6: true,
            driver_opts: HashMap::from([("name".to_string(), "value")]),
        };

        let state = NetworkState::from(&net);

        assert_eq!(state.id.as_ref().unwrap(), net.id.as_ref().unwrap());
        assert_eq!(state.name, net.name);
        assert_eq!(state.driver, net.driver);
        assert_eq!(state.check_duplicate, net.check_duplicate);
        assert_eq!(state.internal, net.internal);
        assert_eq!(state.enable_ipv6, net.enable_ipv6);

        let hm: HashMap<String, &str> = state
            .driver_opts
            .iter()
            .map(|(k, v)| (k.to_string(), v.as_ref()))
            .collect();

        assert_eq!(hm, net.driver_opts);

        let back = Network::from(state);

        assert_eq!(back, net);
    }
}
