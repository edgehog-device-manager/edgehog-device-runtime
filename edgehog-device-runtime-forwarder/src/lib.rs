// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

#![warn(missing_docs, rustdoc::missing_crate_level_docs)]

//! Edgehog Device Runtime Forwarder
//!
//! Implement forwarder functionality on a device.

pub mod astarte;
pub mod collection;
pub mod connection;
pub mod connections_manager;
mod messages;

// re-exported dependencies
pub use astarte_device_sdk;

#[cfg(feature = "_test-utils")]
pub mod test_utils;
