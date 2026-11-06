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

/// Network status event
#[derive(Debug, Clone, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct NetworkEvent {
    pub(crate) status: NetworkEventStatus,
    pub(crate) message: String,
    pub(crate) add_info: Vec<String>,
}

impl DatastreamEvent for NetworkEvent {
    const INTERFACE: &str = "io.edgehog.devicemanager.apps.NetworkEvent";

    type Status = NetworkEventStatus;

    fn new(status: Self::Status, message: impl Into<String>, add_info: Vec<String>) -> Self {
        Self {
            status,
            message: message.into(),
            add_info,
        }
    }
}

impl Display for NetworkEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "deployment event ({}) with message {}",
            self.status, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NetworkEventStatus {
    Create,
    Connect,
    Disconnect,
    Destroy,
    Update,
    Remove,
    Prune,
    Error,
}

impl Display for NetworkEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkEventStatus::Create => write!(f, "Create"),
            NetworkEventStatus::Connect => write!(f, "Connect"),
            NetworkEventStatus::Disconnect => write!(f, "Disconnect"),
            NetworkEventStatus::Destroy => write!(f, "Destroy"),
            NetworkEventStatus::Update => write!(f, "Update"),
            NetworkEventStatus::Remove => write!(f, "Remove"),
            NetworkEventStatus::Prune => write!(f, "Prune"),
            NetworkEventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<NetworkEventStatus> for AstarteData {
    fn from(value: NetworkEventStatus) -> Self {
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
                predicate::eq("io.edgehog.devicemanager.apps.NetworkEvent"),
                predicate::eq(exp_p),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Create")),
                    ("message".to_string(), AstarteData::from("")),
                    (
                        "addInfo".to_string(),
                        AstarteData::from(Vec::<String>::new()),
                    ),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let event = NetworkEvent::with_message(NetworkEventStatus::Create, "");

        event.send(&id, &mut client).await;
    }

    #[rstest::rstest]
    #[case(NetworkEventStatus::Create)]
    #[case(NetworkEventStatus::Connect)]
    #[case(NetworkEventStatus::Disconnect)]
    #[case(NetworkEventStatus::Destroy)]
    #[case(NetworkEventStatus::Update)]
    #[case(NetworkEventStatus::Remove)]
    #[case(NetworkEventStatus::Prune)]
    #[case(NetworkEventStatus::Error)]
    fn should_convert_event_to_string(#[case] status: NetworkEventStatus, #[context] ctx: Context) {
        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, status);
        });
    }
}
