// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

//! Events sent from the device to Astarte

use std::fmt::Display;

use astarte_device_sdk::{AstarteData, IntoAstarteObject};

use super::DatastreamEvent;

/// Image status event
#[derive(Debug, Clone, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ImageEvent {
    pub(crate) status: ImageEventStatus,
    pub(crate) message: String,
    pub(crate) add_info: Vec<String>,
}

impl DatastreamEvent for ImageEvent {
    const INTERFACE: &str = "io.edgehog.devicemanager.apps.ImageEvent";

    type Status = ImageEventStatus;

    fn new(status: Self::Status, message: impl Into<String>, add_info: Vec<String>) -> Self {
        Self {
            status,
            message: message.into(),
            add_info,
        }
    }
}

impl Display for ImageEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "deployment event ({}) with message {}",
            self.status, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageEventStatus {
    Create,
    Delete,
    Import,
    Load,
    Pull,
    Push,
    Save,
    Tag,
    Untag,
    Prune,
    Error,
}

impl Display for ImageEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageEventStatus::Create => write!(f, "Create"),
            ImageEventStatus::Delete => write!(f, "Delete"),
            ImageEventStatus::Import => write!(f, "Import"),
            ImageEventStatus::Load => write!(f, "Load"),
            ImageEventStatus::Pull => write!(f, "Pull"),
            ImageEventStatus::Push => write!(f, "Push"),
            ImageEventStatus::Save => write!(f, "Save"),
            ImageEventStatus::Tag => write!(f, "Tag"),
            ImageEventStatus::Untag => write!(f, "Untag"),
            ImageEventStatus::Prune => write!(f, "Prune"),
            ImageEventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<ImageEventStatus> for AstarteData {
    fn from(value: ImageEventStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_test_utils::with_insta;
    use mockall::predicate;
    use rstest::Context;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_send_event() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();

        let exp_p = format!("/{id}");
        client
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.ImageEvent"),
                predicate::eq(exp_p),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Pull")),
                    ("message".to_string(), AstarteData::from("")),
                    (
                        "addInfo".to_string(),
                        AstarteData::from(Vec::<String>::new()),
                    ),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let event = ImageEvent::with_message(ImageEventStatus::Pull, "");

        event.send(&id, &mut client).await;
    }

    #[rstest::rstest]
    #[case(ImageEventStatus::Create)]
    #[case(ImageEventStatus::Delete)]
    #[case(ImageEventStatus::Import)]
    #[case(ImageEventStatus::Load)]
    #[case(ImageEventStatus::Pull)]
    #[case(ImageEventStatus::Push)]
    #[case(ImageEventStatus::Save)]
    #[case(ImageEventStatus::Tag)]
    #[case(ImageEventStatus::Untag)]
    #[case(ImageEventStatus::Prune)]
    #[case(ImageEventStatus::Error)]
    fn should_convert_event_to_string(#[case] status: ImageEventStatus, #[context] ctx: Context) {
        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, status);
        });
    }
}
