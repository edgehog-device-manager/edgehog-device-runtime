// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Persistent stores of the request issued by Astarte and resources created.

#![allow(dead_code)]

use std::ops::Not;

use edgehog_store::db::{self, HandleError};

type Result<T> = std::result::Result<T, StoreError>;

/// Error returned by the [`StateStore`].
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum StoreError {
    /// database operation failed
    Handle(#[from] HandleError),
}

/// Handle to persist the state.
///
/// The file is a new line delimited JSON.
#[derive(Debug)]
pub struct StateStore {
    handle: db::Handle,
}

impl StateStore {
    /// Creates a new state store
    pub fn new(handle: db::Handle) -> Self {
        Self { handle }
    }

    pub(crate) fn clone_lazy(&self) -> Self {
        Self {
            handle: self.handle.clone_lazy(),
        }
    }
}

fn split_key_value(value: &str) -> Option<(&str, Option<&str>)> {
    value.split_once('=').and_then(|(k, v)| {
        if k.is_empty() {
            return None;
        }

        let v = v.is_empty().not().then_some(v);

        Some((k, v))
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_parse_key_value() {
        let cases = [
            ("device=tmpfs", ("device", Some("tmpfs"))),
            ("o=size=100m,uid=1000", ("o", Some("size=100m,uid=1000"))),
            ("type=tmpfs", ("type", Some("tmpfs"))),
        ];

        for (case, exp) in cases {
            let res = split_key_value(case).unwrap();

            assert_eq!(res, exp);
        }
    }
}
