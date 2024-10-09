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

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::image::Image;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImageState<'a> {
    id: Option<Cow<'a, str>>,
    reference: Cow<'a, str>,
    registry_auth: Option<Cow<'a, str>>,
}

impl<'a, S> From<&'a Image<S>> for ImageState<'a>
where
    S: AsRef<str>,
{
    fn from(value: &'a Image<S>) -> Self {
        Self {
            id: value.id.as_deref().map(Cow::Borrowed),
            reference: Cow::Borrowed(value.reference.as_ref()),
            registry_auth: value
                .registry_auth
                .as_ref()
                .map(|s| Cow::Borrowed(s.as_ref())),
        }
    }
}
