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

#![warn(missing_docs, rustdoc::missing_crate_level_docs)]

//! # Edgehog Device Runtime Docker
//!
//! Library to manage container for the `edgehog-device-runtime`.
//!
//! It will handle communications with the Docker daemon and solve the requests received from
//! Astarte.

pub(crate) mod client;
pub mod docker;
pub mod error;
pub mod image;
mod properties;
pub mod request;
pub mod service;

#[cfg(feature = "mock")]
mod mock;

/// Re-export third parties dependencies
pub use bollard;

/// Re-export internal structs
pub use self::docker::*;
