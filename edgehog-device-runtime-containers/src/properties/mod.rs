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

use astarte_device_sdk::{AstarteType, Error as AstarteError};
use async_trait::async_trait;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub use astarte_device_sdk_mock::Client;
    } else {
        pub use astarte_device_sdk::Client;
    }
}

pub(crate) mod container;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

/// Error from handling the Astarte properties.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum PropError {
    /// endpoint missing prefix
    MissingPrefix,
    /// endpoint missing the id
    MissingId,
    /// couldn't send {path} to Astarte
    Send {
        path: String,
        #[source]
        backtrace: AstarteError,
    },
}

#[async_trait]
pub(crate) trait AvailableProp {
    fn interface() -> &'static str;

    fn id(&self) -> &str;

    async fn send<D>(&self, device: &D) -> Result<(), PropError>
    where
        D: Client + Sync + 'static;

    async fn send_field<D, T>(&self, device: &D, field: &str, data: T) -> Result<(), PropError>
    where
        D: Client + Sync + 'static,
        T: Into<AstarteType> + Send + 'static,
    {
        let interface = Self::interface();
        let endpoint = format!("/{}/{}", self.id(), field);

        device
            .send(interface, &endpoint, data)
            .await
            .map_err(|err| PropError::Send {
                path: format!("{interface}{endpoint}"),
                backtrace: err,
            })
    }
}
