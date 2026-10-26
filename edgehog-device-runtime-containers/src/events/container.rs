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

/// Container status event
#[derive(Debug, Clone, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ContainerEvent {
    pub(crate) status: ContainerEventStatus,
    pub(crate) message: String,
    pub(crate) add_info: Vec<String>,
}

impl DatastreamEvent for ContainerEvent {
    const INTERFACE: &str = "io.edgehog.devicemanager.apps.ContainerEvent";

    type Status = ContainerEventStatus;

    fn new(status: Self::Status, message: impl Into<String>, add_info: Vec<String>) -> Self {
        Self {
            status,
            message: message.into(),
            add_info,
        }
    }
}

impl Display for ContainerEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "deployment event ({}) with message {}",
            self.status, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerEventStatus {
    Attach,
    Commit,
    Copy,
    Create,
    Destroy,
    Detach,
    Die,
    ExecCreate,
    ExecDetach,
    ExecStart,
    ExecDie,
    Export,
    HealthStatus,
    Kill,
    Oom,
    Pause,
    Rename,
    Resize,
    Restart,
    Start,
    Stop,
    Top,
    Unpause,
    Update,
    Prune,
    Error,
}

impl Display for ContainerEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerEventStatus::Attach => write!(f, "Attach"),
            ContainerEventStatus::Commit => write!(f, "Commit"),
            ContainerEventStatus::Copy => write!(f, "Copy"),
            ContainerEventStatus::Create => write!(f, "Create"),
            ContainerEventStatus::Destroy => write!(f, "Destroy"),
            ContainerEventStatus::Detach => write!(f, "Detach"),
            ContainerEventStatus::Die => write!(f, "Die"),
            ContainerEventStatus::ExecCreate => write!(f, "ExecCreate"),
            ContainerEventStatus::ExecDetach => write!(f, "ExecDetach"),
            ContainerEventStatus::ExecStart => write!(f, "ExecStart"),
            ContainerEventStatus::ExecDie => write!(f, "ExecDie"),
            ContainerEventStatus::Export => write!(f, "Export"),
            ContainerEventStatus::HealthStatus => write!(f, "HealthStatus"),
            ContainerEventStatus::Kill => write!(f, "Kill"),
            ContainerEventStatus::Oom => write!(f, "Oom"),
            ContainerEventStatus::Pause => write!(f, "Pause"),
            ContainerEventStatus::Rename => write!(f, "Rename"),
            ContainerEventStatus::Resize => write!(f, "Resize"),
            ContainerEventStatus::Restart => write!(f, "Restart"),
            ContainerEventStatus::Start => write!(f, "Start"),
            ContainerEventStatus::Stop => write!(f, "Stop"),
            ContainerEventStatus::Top => write!(f, "Top"),
            ContainerEventStatus::Unpause => write!(f, "Unpause"),
            ContainerEventStatus::Update => write!(f, "Update"),
            ContainerEventStatus::Prune => write!(f, "Prune"),
            ContainerEventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<ContainerEventStatus> for AstarteData {
    fn from(value: ContainerEventStatus) -> Self {
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
                predicate::eq("io.edgehog.devicemanager.apps.ContainerEvent"),
                predicate::eq(exp_p),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Start")),
                    ("message".to_string(), AstarteData::from("")),
                    (
                        "addInfo".to_string(),
                        AstarteData::from(Vec::<String>::new()),
                    ),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let event = ContainerEvent::with_message(ContainerEventStatus::Start, "");

        event.send(&id, &mut client).await;
    }

    #[rstest::rstest]
    #[case(ContainerEventStatus::Attach)]
    #[case(ContainerEventStatus::Commit)]
    #[case(ContainerEventStatus::Copy)]
    #[case(ContainerEventStatus::Create)]
    #[case(ContainerEventStatus::Destroy)]
    #[case(ContainerEventStatus::Detach)]
    #[case(ContainerEventStatus::Die)]
    #[case(ContainerEventStatus::ExecCreate)]
    #[case(ContainerEventStatus::ExecDetach)]
    #[case(ContainerEventStatus::ExecStart)]
    #[case(ContainerEventStatus::ExecDie)]
    #[case(ContainerEventStatus::Export)]
    #[case(ContainerEventStatus::HealthStatus)]
    #[case(ContainerEventStatus::Kill)]
    #[case(ContainerEventStatus::Oom)]
    #[case(ContainerEventStatus::Pause)]
    #[case(ContainerEventStatus::Rename)]
    #[case(ContainerEventStatus::Resize)]
    #[case(ContainerEventStatus::Restart)]
    #[case(ContainerEventStatus::Start)]
    #[case(ContainerEventStatus::Stop)]
    #[case(ContainerEventStatus::Top)]
    #[case(ContainerEventStatus::Unpause)]
    #[case(ContainerEventStatus::Update)]
    #[case(ContainerEventStatus::Prune)]
    #[case(ContainerEventStatus::Error)]
    fn should_convert_event_to_string(
        #[case] status: ContainerEventStatus,
        #[context] ctx: Context,
    ) {
        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, status);
        });
    }
}
