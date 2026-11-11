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

//! Container image policy request from Astarte.

use std::str::FromStr;

use astarte_device_sdk::{
    DeviceEvent, FromEvent, Value,
    astarte_device_error::Error,
    error::{AstarteError, InterfaceError},
    event::FromEventError,
    properties::PropAccess,
    store::StoredProp,
};
use tracing::warn;

use crate::service::image_policy::Sha256Digest;

/// Image policy property update received from Astarte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePolicy {
    pub(crate) registry_id: Sha256Digest,
    pub(crate) mapping: ImagePolicyMapping,
}

impl ImagePolicy {
    /// Interface name for `io.edgehog.devicemanager.apps.ImagePolicy`.
    pub const INTERFACE: &str = "io.edgehog.devicemanager.apps.ImagePolicy";

    /// Converts a stored Astarte property into an [`ImagePolicy`] event.
    pub(crate) fn from_stored(prop: StoredProp) -> Result<Self, Error<FromEventError>> {
        let event = DeviceEvent {
            interface: prop.interface,
            path: prop.path,
            data: Value::Property(Some(prop.value)),
        };

        Self::from_event(event)
    }

    pub(crate) async fn fetch<D>(device: &mut D) -> Result<Vec<Self>, AstarteError>
    where
        D: astarte_device_sdk::Client + PropAccess + Send + Sync + 'static,
    {
        let vec = device
            .interface_props(Self::INTERFACE)
            .await?
            .into_iter()
            .map(ImagePolicy::from_stored)
            .filter_map(|p| {
                p.inspect_err(|error| {
                    warn!(
                        %error,
                        "ignoring invalid stored ImagePolicy property"
                    )
                })
                .ok()
            })
            .collect();

        Ok(vec)
    }
}

impl FromEvent for ImagePolicy {
    type Err = Error<FromEventError>;

    fn from_event(event: DeviceEvent) -> Result<Self, Self::Err> {
        let registry_id = event
            .path
            .strip_prefix("/policy/")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(id, field)| !id.is_empty() && !field.contains('/'))
            .and_then(|(id, _)| Sha256Digest::from_str(id).ok())
            .ok_or_else(|| {
                Error::with(
                    FromEventError::Interface(InterfaceError::Path),
                    "not a valid image policy path",
                )
                .set_ctx(event.path.clone())
            })?;

        let mapping = ImagePolicyMapping::from_event(event)?;

        Ok(Self {
            registry_id,
            mapping,
        })
    }
}

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.ImagePolicy",
    interface_type = "properties",
    aggregation = "individual"
)]
pub(crate) enum ImagePolicyMapping {
    #[mapping(endpoint = "/policy/%{registry_id}/repository", allow_unset = true)]
    Repository(Option<String>),
    #[mapping(endpoint = "/policy/%{registry_id}/type", allow_unset = true)]
    Type(Option<String>),
    #[mapping(endpoint = "/policy/%{registry_id}/keyId", allow_unset = true)]
    KeyId(Option<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyRequirement {
    Reject,
    SignedBy,
}

impl FromStr for PolicyRequirement {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Reject" => Ok(Self::Reject),
            "SignedBy" => Ok(Self::SignedBy),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::astarte_interfaces::schema::Ownership;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    pub(crate) fn registry_id_for(scope: &str) -> Sha256Digest {
        Sha256Digest::hash(scope.as_bytes())
    }

    pub(crate) fn image_policy_event(path: &str, value: Option<&str>) -> DeviceEvent {
        DeviceEvent {
            interface: ImagePolicy::INTERFACE.to_string(),
            path: path.to_string(),
            data: Value::Property(value.map(|v| AstarteData::String(v.to_string()))),
        }
    }

    #[rstest]
    #[case(
        "repository",
        Some("quay.io/my-org"),
        ImagePolicyMapping::Repository(Some("quay.io/my-org".to_string()))
    )]
    #[case("repository", None, ImagePolicyMapping::Repository(None))]
    #[case(
        "type",
        Some("Reject"),
        ImagePolicyMapping::Type(Some("Reject".to_string()))
    )]
    #[case(
        "type",
        Some("SignedBy"),
        ImagePolicyMapping::Type(Some("SignedBy".to_string()))
    )]
    #[case("type", None, ImagePolicyMapping::Type(None))]
    #[case(
        "keyId",
        Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ImagePolicyMapping::KeyId(Some(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
        ))
    )]
    #[case("keyId", None, ImagePolicyMapping::KeyId(None))]
    fn should_parse_image_policy_event(
        #[case] field: &str,
        #[case] value: Option<&str>,
        #[case] expected_mapping: ImagePolicyMapping,
    ) {
        let registry_id = registry_id_for("quay.io/my-org");
        let path = format!("/policy/{registry_id}/{field}");
        let event = image_policy_event(&path, value);
        let parsed = ImagePolicy::from_event(event).unwrap();

        assert_eq!(
            parsed,
            ImagePolicy {
                registry_id,
                mapping: expected_mapping,
            }
        );
    }

    #[rstest]
    #[case("/policy//type", Some("Reject"))]
    #[case("/policy/abc123/type", Some("Reject"))]
    #[case("/policy/923f765a-3bbb-4252-be88-b2c85e1d9911/type", Some("Reject"))]
    #[case(
        "/policy/0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF/type",
        Some("Reject")
    )]
    #[case(
        "/wrong/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/type",
        Some("Reject")
    )]
    #[case(
        "/policy/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/extra/type",
        Some("Reject")
    )]
    #[case(
        "/policy/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/unknown",
        Some("Reject")
    )]
    #[case(
        "/policy/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        Some("Reject")
    )]
    fn should_reject_invalid_paths(#[case] path: &str, #[case] value: Option<&str>) {
        let event = image_policy_event(path, value);

        assert!(ImagePolicy::from_event(event).is_err());
    }

    #[test]
    fn should_convert_from_stored_prop() {
        let registry_id = registry_id_for("docker.io");
        let stored = StoredProp {
            interface: ImagePolicy::INTERFACE.to_string(),
            path: format!("/policy/{registry_id}/type"),
            value: AstarteData::String("Reject".to_string()),
            interface_major: 0,
            ownership: Ownership::Server,
        };

        let policy = ImagePolicy::from_stored(stored).unwrap();

        assert_eq!(
            policy,
            ImagePolicy {
                registry_id,
                mapping: ImagePolicyMapping::Type(Some("Reject".to_string())),
            }
        );
    }
}
