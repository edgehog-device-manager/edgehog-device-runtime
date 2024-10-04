// This file is part of Edgehog.
//
// Copyright 2023-2024 SECO Mind Srl
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

//! Container properties sent from the device to Astarte.

use std::num::ParseIntError;

use astarte_device_sdk::{types::TypeError, Error as AstarteError};

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
    /// couldn't property into {into}, since it's missing the field {field}
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
    /// couldn't send {path} to Astarte
    Send {
        path: String,
        #[source]
        backtrace: AstarteError,
    },
}

/// Error from parsing a binding
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum BindingError {
    /// couldn't parse {binding} port {value}
    Port {
        /// Binding received
        binding: &'static str,
        /// Port of the binding
        value: String,
        /// Error converting the port
        #[source]
        source: ParseIntError,
    },
}
