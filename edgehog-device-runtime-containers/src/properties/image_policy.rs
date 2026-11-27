// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Container image registry and repository pull policy properties.

use std::collections::HashSet;
use std::fmt::Display;
use std::str::FromStr;

use astarte_device_sdk::AstarteData;
use astarte_device_sdk::error::AstarteError;
use astarte_device_sdk::properties::PropAccess;
use tracing::{error, warn};

use super::Client;
use crate::service::image_policy::Sha256Digest;

/// Policy action applied to matching container image repositories.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContainerPolicy {
    /// Accept and pull images without signature verification.
    #[default]
    InsecureAcceptAnything,
    /// Reject pulling images matching this scope.
    Reject,
}

impl ContainerPolicy {
    /// Returns `true` if pulling an image is allowed under this policy.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::InsecureAcceptAnything)
    }
}

/// Status of a policy rule reported on `io.edgehog.devicemanager.apps.AvailableImagePolicies`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImagePolicyStatus {
    Present,
    Error,
    Missing,
}

impl Display for ImagePolicyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImagePolicyStatus::Present => write!(f, "Present"),
            ImagePolicyStatus::Error => write!(f, "Error"),
            ImagePolicyStatus::Missing => write!(f, "Missing"),
        }
    }
}

impl From<ImagePolicyStatus> for AstarteData {
    fn from(value: ImagePolicyStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

/// Available image policy property on `io.edgehog.devicemanager.apps.AvailableImagePolicies`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableImagePolicy {
    id: Sha256Digest,
}

impl AvailableImagePolicy {
    pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.apps.AvailableImagePolicies";

    pub(crate) const fn new(id: Sha256Digest) -> Self {
        Self { id }
    }

    /// Parses the registry id from the path of an `AvailableImagePolicies` property.
    fn parse_available_id(path: &str) -> Option<Sha256Digest> {
        path.strip_prefix('/')
            .and_then(|rest| rest.strip_suffix("/status"))
            .and_then(|id| Sha256Digest::from_str(id).ok())
    }

    pub(crate) async fn send<D>(self, device: &mut D, data: ImagePolicyStatus)
    where
        D: Client + Send + Sync + 'static,
    {
        let endpoint = format!("/{}/status", self.id);

        if let Err(err) = device
            .set_property(Self::INTERFACE, &endpoint, data.into())
            .await
        {
            error!(
                error = format!("{:#}", eyre::Report::new(err)),
                interface = Self::INTERFACE,
                endpoint,
                "couldn't send image policy status"
            );
        }
    }

    pub(crate) async fn unset<D>(&self, device: &mut D)
    where
        D: Client + Send + Sync + 'static,
    {
        let endpoint = format!("/{}/status", self.id);

        if let Err(err) = device.unset_property(Self::INTERFACE, &endpoint).await {
            error!(
                error = format!("{:#}", eyre::Report::new(err)),
                interface = Self::INTERFACE,
                endpoint,
                "couldn't unset image policy status"
            );
        }
    }

    pub(crate) async fn fetch<D>(device: &mut D) -> Result<HashSet<Sha256Digest>, AstarteError>
    where
        D: astarte_device_sdk::Client + PropAccess + Send + Sync + 'static,
    {
        let set = device
            .interface_props(Self::INTERFACE)
            .await?
            .into_iter()
            .filter_map(|e| {
                let path = &e.path;
                let id = Self::parse_available_id(path);

                if id.is_none() {
                    warn!(%path, "parsed stored property did not map to a valid sha256 digest");
                }

                id
            })
            .collect();

        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::astarte_interfaces::schema::Ownership;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::{SqliteStore, StoredProp};
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use astarte_device_sdk_mock::mockall::Sequence;
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(ImagePolicyStatus::Present, "Present")]
    #[case(ImagePolicyStatus::Error, "Error")]
    #[case(ImagePolicyStatus::Missing, "Missing")]
    fn should_convert_status_to_astarte_data(
        #[case] status: ImagePolicyStatus,
        #[case] expected: &str,
    ) {
        assert_eq!(
            AstarteData::from(status),
            AstarteData::String(expected.to_string())
        );
    }

    #[tokio::test]
    async fn should_send_unset_and_fetch_available_image_policy() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        let id = Sha256Digest::hash(b"docker.io");
        let prop = AvailableImagePolicy::new(id);

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq(AvailableImagePolicy::INTERFACE),
                predicate::eq(format!("/{id}/status")),
                predicate::eq(AstarteData::from("Present")),
            )
            .returning(|_, _, _| Ok(()));

        client
            .expect_unset_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq(AvailableImagePolicy::INTERFACE),
                predicate::eq(format!("/{id}/status")),
            )
            .returning(|_, _| Ok(()));

        client
            .expect_interface_props()
            .once()
            .in_sequence(&mut seq)
            .with(predicate::eq(AvailableImagePolicy::INTERFACE))
            .returning(move |_| {
                Ok(vec![
                    StoredProp {
                        interface: AvailableImagePolicy::INTERFACE.to_string(),
                        path: format!("/{id}/status"),
                        value: AstarteData::String("Present".to_string()),
                        interface_major: 0,
                        ownership: Ownership::Device,
                    },
                    StoredProp {
                        interface: AvailableImagePolicy::INTERFACE.to_string(),
                        path: "/invalid-id/status".to_string(),
                        value: AstarteData::String("Error".to_string()),
                        interface_major: 0,
                        ownership: Ownership::Device,
                    },
                ])
            });

        prop.send(&mut client, ImagePolicyStatus::Present).await;
        prop.unset(&mut client).await;

        let fetched = AvailableImagePolicy::fetch(&mut client).await.unwrap();
        assert_eq!(fetched, HashSet::from([id]));
    }
}
