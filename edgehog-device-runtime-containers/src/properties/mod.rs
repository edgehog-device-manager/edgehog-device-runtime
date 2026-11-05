// This file is part of Edgehog.
//
// Copyright 2023-2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
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
use tracing::error;
use uuid::Uuid;

pub use astarte_device_sdk::Client;

pub(crate) mod container;
pub(crate) mod deployment;
pub(crate) mod device_mapping;
pub(crate) mod device_request;
pub(crate) mod env_file;
pub(crate) mod file_bind;
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

pub(crate) trait AvailableProp {
    type Data: Into<AstarteData> + Send + 'static;

    fn interface() -> &'static str;

    fn id(&self) -> &Uuid;

    /// Only updates the status and not the other fields
    async fn send_status<D>(self, device: &mut D, data: Self::Data) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static;

    /// Send all the fields of the property.
    async fn send<D>(self, device: &mut D, data: Self::Data) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static;

    /// Unsets all the fields of the property.
    async fn unset<D>(&self, device: &mut D) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static;

    async fn send_opt_field<D, T>(
        id: &Uuid,
        device: &mut D,
        field: &str,
        data: Option<T>,
    ) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
        T: Into<AstarteData> + Send + 'static,
    {
        match data {
            Some(data) => Self::send_field(id, device, field, data).await,
            None => Self::unset_field(id, device, field).await,
        }
    }

    async fn send_field<D, T>(
        id: &Uuid,
        device: &mut D,
        field: &str,
        data: T,
    ) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
        T: Into<AstarteData> + Send + 'static,
    {
        let interface = Self::interface();
        let endpoint = format!("/{}/{}", id, field);

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

    async fn unset_field<D>(id: &Uuid, device: &mut D, field: &str) -> Result<(), PropertyError>
    where
        D: Client + Send + Sync + 'static,
    {
        let interface = Self::interface();
        let endpoint = format!("/{}/{}", id, field);

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

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::{MockDeviceClient, mockall::Sequence};
    use mockall::predicate;

    use super::*;

    pub(crate) fn expect_send_field<T>(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        interface: &'static str,
        id: &Uuid,
        field: &str,
        data: T,
    ) where
        T: Into<AstarteData>,
    {
        client
            .expect_set_property()
            .once()
            .in_sequence(seq)
            .with(
                predicate::eq(interface),
                predicate::eq(format!("/{id}/{field}")),
                predicate::eq(data.into()),
            )
            .returning(|_, _, _| Ok(()));
    }

    pub(crate) fn expect_unset_field(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        interface: &'static str,
        id: &Uuid,
        field: &str,
    ) {
        client
            .expect_unset_property()
            .once()
            .in_sequence(seq)
            .with(
                predicate::eq(interface),
                predicate::eq(format!("/{id}/{field}")),
            )
            .returning(|_, _| Ok(()));
    }

    pub(crate) fn expect_send_opt_field<T>(
        client: &mut MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        seq: &mut Sequence,
        interface: &'static str,
        id: &Uuid,
        field: &str,
        data: Option<T>,
    ) where
        T: Into<AstarteData>,
    {
        match data {
            Some(data) => expect_send_field(client, seq, interface, id, field, data),
            None => expect_unset_field(client, seq, interface, id, field),
        }
    }
}
