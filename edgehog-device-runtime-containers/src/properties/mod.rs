// This file is part of Edgehog.
//
// Copyright 2023 - 2025 SECO Mind Srl
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

//! Container properties sent from the device to Astarte.

use astarte_device_sdk::AstarteData;
use async_trait::async_trait;
use tracing::error;
use uuid::Uuid;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        pub use astarte_device_sdk_mock::Client;
    } else {
        pub use astarte_device_sdk::Client;
    }
}

pub(crate) mod container;
pub(crate) mod deployment;
pub(crate) mod device_mapping;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

/// Error returned when failing to publish a property
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum PropertyError {
    /// couldn't send property {interface}{endpoint}
    Send {
        /// Interface of the property
        interface: String,
        /// Endpoint of the property
        endpoint: String,
    },
    /// couldn't unset property {interface}{endpoint}
    Unset {
        /// Interface of the property
        interface: String,
        /// Endpoint of the property
        endpoint: String,
    },
}

#[async_trait]
pub(crate) trait AvailableProp {
    type Data: Into<AstarteData> + Send + 'static;

    fn interface() -> &'static str;

    fn field() -> &'static str;

    fn id(&self) -> &Uuid;

    async fn send<D>(&self, device: &mut D, data: Self::Data) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        self.send_field(device, Self::field(), data).await
    }

    async fn send_field<D, T>(
        &self,
        device: &mut D,
        field: &str,
        data: T,
    ) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
        T: Into<AstarteData> + Send + 'static,
    {
        let interface = Self::interface();
        let endpoint = format!("/{}/{}", self.id(), field);

        device
            .set_property(interface, &endpoint, data.into())
            .await
            .map_err(|err| {
                error!(
                    error = format!("{:#}", eyre::Report::new(err)),
                    "couldn't send data for {interface}{endpoint}"
                );

                PropertyError::Send {
                    interface: interface.to_string(),
                    endpoint,
                }
            })
    }

    async fn unset<D>(&self, device: &mut D) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        let interface = Self::interface();
        let field = Self::field();
        let endpoint = format!("/{}/{}", self.id(), field);

        device
            .unset_property(interface, &endpoint)
            .await
            .map_err(|err| {
                error!(
                    error = format!("{:#}", eyre::Report::new(err)),
                    "couldn't unset data for {interface}{endpoint}"
                );

                PropertyError::Unset {
                    interface: interface.to_string(),
                    endpoint,
                }
            })
    }
}
