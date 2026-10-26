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

//! Events of the container reources.
//!
//! Various objects within Docker report events when something happens to them.
//!
//! Containers report these events: attach, commit, copy, create, destroy, detach, die, exec_create,
//! exec_detach, exec_start, exec_die, export, health_status, kill, oom, pause, rename, resize,
//! restart, start, stop, top, unpause, update, and prune
//!
//! Images report these events: create, delete, import, load, pull, push, save, tag, untag, and
//! prune
//!
//! Volumes report these events: create, mount, unmount, destroy, and prune
//!
//! Networks report these events: create, connect, disconnect, destroy, update, remove, and prune
//!
//! The Builder reports prune events

use astarte_device_sdk::chrono::{DateTime, Utc};
use bollard::models::EventMessage;
use bollard::plugin::EventMessageTypeEnum;
use tracing::debug;

use crate::events::container::ContainerEventStatus;
use crate::events::image::ImageEventStatus;
use crate::events::network::NetworkEventStatus;
use crate::events::volume::VolumeEventStatus;

/// Container runtime events
#[derive(Debug)]
pub struct ContainerRuntimeEvent {
    pub(crate) inner: EventMessage,
}

impl ContainerRuntimeEvent {
    pub(crate) fn timestamp(&self) -> Option<DateTime<Utc>> {
        self.inner
            .time_nano
            .map(DateTime::from_timestamp_nanos)
            .or_else(|| self.inner.time.and_then(DateTime::from_timestamp_secs))
    }

    pub(crate) fn get_local_id(&self) -> Option<&str> {
        self.inner
            .actor
            .as_ref()
            .and_then(|actor| actor.id.as_deref())
    }

    pub(crate) fn get_type(&self) -> Option<EventMessageTypeEnum> {
        self.inner.typ
    }

    pub(crate) fn container_action(&self) -> Option<ContainerEventStatus> {
        let is_not_container = self
            .get_type()
            .is_none_or(|typ| typ != EventMessageTypeEnum::CONTAINER);

        if is_not_container {
            debug!("event not for container");

            return None;
        }

        let action = self.inner.action.as_deref().unwrap_or_default();

        match action {
            "attach" => Some(ContainerEventStatus::Attach),
            "commit" => Some(ContainerEventStatus::Commit),
            "copy" => Some(ContainerEventStatus::Copy),
            "create" => Some(ContainerEventStatus::Create),
            "destroy" => Some(ContainerEventStatus::Destroy),
            "detach" => Some(ContainerEventStatus::Detach),
            "die" => Some(ContainerEventStatus::Die),
            "exec_create" => Some(ContainerEventStatus::ExecCreate),
            "exec_detach" => Some(ContainerEventStatus::ExecDetach),
            "exec_start" => Some(ContainerEventStatus::ExecStart),
            "exec_die" => Some(ContainerEventStatus::ExecDie),
            "export" => Some(ContainerEventStatus::Export),
            "health_status" => Some(ContainerEventStatus::HealthStatus),
            "kill" => Some(ContainerEventStatus::Kill),
            "oom" => Some(ContainerEventStatus::Oom),
            "pause" => Some(ContainerEventStatus::Pause),
            "rename" => Some(ContainerEventStatus::Rename),
            "resize" => Some(ContainerEventStatus::Resize),
            "restart" => Some(ContainerEventStatus::Restart),
            "start" => Some(ContainerEventStatus::Start),
            "stop" => Some(ContainerEventStatus::Stop),
            "top" => Some(ContainerEventStatus::Top),
            "unpause" => Some(ContainerEventStatus::Unpause),
            "update" => Some(ContainerEventStatus::Update),
            "prune" => Some(ContainerEventStatus::Prune),
            _ => {
                debug!(action, "unhandled container event");

                None
            }
        }
    }

    pub(crate) fn image_action(&self) -> Option<ImageEventStatus> {
        let is_not_image = self
            .inner
            .typ
            .is_none_or(|typ| typ != EventMessageTypeEnum::IMAGE);

        if is_not_image {
            debug!("event not for image");

            return None;
        }

        let action = self.inner.action.as_deref().unwrap_or_default();

        match action {
            "create" => Some(ImageEventStatus::Create),
            "delete" => Some(ImageEventStatus::Delete),
            "import" => Some(ImageEventStatus::Import),
            "load" => Some(ImageEventStatus::Load),
            "pull" => Some(ImageEventStatus::Pull),
            "push" => Some(ImageEventStatus::Push),
            "save" => Some(ImageEventStatus::Save),
            "tag" => Some(ImageEventStatus::Tag),
            "untag" => Some(ImageEventStatus::Untag),
            "prune" => Some(ImageEventStatus::Prune),
            _ => {
                debug!(action, "unhandled image event");

                None
            }
        }
    }

    pub(crate) fn network_action(&self) -> Option<NetworkEventStatus> {
        let is_not_network = self
            .inner
            .typ
            .is_none_or(|typ| typ != EventMessageTypeEnum::NETWORK);

        if is_not_network {
            debug!("event not for network");

            return None;
        }

        let action = self.inner.action.as_deref().unwrap_or_default();

        match action {
            "create" => Some(NetworkEventStatus::Create),
            "connect" => Some(NetworkEventStatus::Connect),
            "disconnect" => Some(NetworkEventStatus::Disconnect),
            "destroy" => Some(NetworkEventStatus::Destroy),
            "update" => Some(NetworkEventStatus::Update),
            "remove" => Some(NetworkEventStatus::Remove),
            "prune" => Some(NetworkEventStatus::Prune),
            _ => {
                debug!(action, "unhandled network event");

                None
            }
        }
    }

    pub(crate) fn volume_action(&self) -> Option<VolumeEventStatus> {
        let is_not_volume = self
            .inner
            .typ
            .is_none_or(|typ| typ != EventMessageTypeEnum::VOLUME);

        if is_not_volume {
            debug!("event not for volume");

            return None;
        }

        let action = self.inner.action.as_deref().unwrap_or_default();

        match action {
            "create" => Some(VolumeEventStatus::Create),
            "mount" => Some(VolumeEventStatus::Mount),
            "unmount" => Some(VolumeEventStatus::Unmount),
            "destroy" => Some(VolumeEventStatus::Destroy),
            "prune" => Some(VolumeEventStatus::Prune),
            _ => {
                debug!(action, "unhandled volume event");

                None
            }
        }
    }
}

impl From<EventMessage> for ContainerRuntimeEvent {
    fn from(value: EventMessage) -> Self {
        ContainerRuntimeEvent { inner: value }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn should_extract_timestamp() {
        let event = ContainerRuntimeEvent {
            inner: EventMessage {
                time: Some(1_700_000_000),
                time_nano: Some(1_700_000_000_123_456_789),
                ..Default::default()
            },
        };
        assert_eq!(
            event.timestamp(),
            Some(DateTime::from_timestamp_nanos(1_700_000_000_123_456_789))
        );

        let event_secs = ContainerRuntimeEvent {
            inner: EventMessage {
                time: Some(1_700_000_000),
                time_nano: None,
                ..Default::default()
            },
        };
        assert_eq!(
            event_secs.timestamp(),
            DateTime::from_timestamp_secs(1_700_000_000)
        );

        let event_none = ContainerRuntimeEvent {
            inner: EventMessage::default(),
        };
        assert_eq!(event_none.timestamp(), None);
    }

    #[rstest]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("attach"),
        Some(ContainerEventStatus::Attach)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("commit"),
        Some(ContainerEventStatus::Commit)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("copy"),
        Some(ContainerEventStatus::Copy)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("create"),
        Some(ContainerEventStatus::Create)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("destroy"),
        Some(ContainerEventStatus::Destroy)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("detach"),
        Some(ContainerEventStatus::Detach)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("die"),
        Some(ContainerEventStatus::Die)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("exec_create"),
        Some(ContainerEventStatus::ExecCreate)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("exec_detach"),
        Some(ContainerEventStatus::ExecDetach)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("exec_start"),
        Some(ContainerEventStatus::ExecStart)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("exec_die"),
        Some(ContainerEventStatus::ExecDie)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("export"),
        Some(ContainerEventStatus::Export)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("health_status"),
        Some(ContainerEventStatus::HealthStatus)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("kill"),
        Some(ContainerEventStatus::Kill)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("oom"),
        Some(ContainerEventStatus::Oom)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("pause"),
        Some(ContainerEventStatus::Pause)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("rename"),
        Some(ContainerEventStatus::Rename)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("resize"),
        Some(ContainerEventStatus::Resize)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("restart"),
        Some(ContainerEventStatus::Restart)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("start"),
        Some(ContainerEventStatus::Start)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("stop"),
        Some(ContainerEventStatus::Stop)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("top"),
        Some(ContainerEventStatus::Top)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("unpause"),
        Some(ContainerEventStatus::Unpause)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("update"),
        Some(ContainerEventStatus::Update)
    )]
    #[case(
        Some(EventMessageTypeEnum::CONTAINER),
        Some("prune"),
        Some(ContainerEventStatus::Prune)
    )]
    #[case(Some(EventMessageTypeEnum::CONTAINER), Some("unknown"), None)]
    #[case(Some(EventMessageTypeEnum::CONTAINER), None, None)]
    #[case(Some(EventMessageTypeEnum::IMAGE), Some("create"), None)]
    #[case(None, Some("create"), None)]
    fn should_parse_container_action(
        #[case] typ: Option<EventMessageTypeEnum>,
        #[case] action: Option<&str>,
        #[case] expected: Option<ContainerEventStatus>,
    ) {
        let event = ContainerRuntimeEvent {
            inner: EventMessage {
                typ,
                action: action.map(str::to_string),
                ..Default::default()
            },
        };

        assert_eq!(event.container_action(), expected);
    }

    #[rstest]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("create"),
        Some(ImageEventStatus::Create)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("delete"),
        Some(ImageEventStatus::Delete)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("import"),
        Some(ImageEventStatus::Import)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("load"),
        Some(ImageEventStatus::Load)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("pull"),
        Some(ImageEventStatus::Pull)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("push"),
        Some(ImageEventStatus::Push)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("save"),
        Some(ImageEventStatus::Save)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("tag"),
        Some(ImageEventStatus::Tag)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("untag"),
        Some(ImageEventStatus::Untag)
    )]
    #[case(
        Some(EventMessageTypeEnum::IMAGE),
        Some("prune"),
        Some(ImageEventStatus::Prune)
    )]
    #[case(Some(EventMessageTypeEnum::IMAGE), Some("unknown"), None)]
    #[case(Some(EventMessageTypeEnum::IMAGE), None, None)]
    #[case(Some(EventMessageTypeEnum::CONTAINER), Some("pull"), None)]
    #[case(None, Some("pull"), None)]
    fn should_parse_image_action(
        #[case] typ: Option<EventMessageTypeEnum>,
        #[case] action: Option<&str>,
        #[case] expected: Option<ImageEventStatus>,
    ) {
        let event = ContainerRuntimeEvent {
            inner: EventMessage {
                typ,
                action: action.map(str::to_string),
                ..Default::default()
            },
        };

        assert_eq!(event.image_action(), expected);
    }

    #[rstest]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("create"),
        Some(NetworkEventStatus::Create)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("connect"),
        Some(NetworkEventStatus::Connect)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("disconnect"),
        Some(NetworkEventStatus::Disconnect)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("destroy"),
        Some(NetworkEventStatus::Destroy)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("update"),
        Some(NetworkEventStatus::Update)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("remove"),
        Some(NetworkEventStatus::Remove)
    )]
    #[case(
        Some(EventMessageTypeEnum::NETWORK),
        Some("prune"),
        Some(NetworkEventStatus::Prune)
    )]
    #[case(Some(EventMessageTypeEnum::NETWORK), Some("unknown"), None)]
    #[case(Some(EventMessageTypeEnum::NETWORK), None, None)]
    #[case(Some(EventMessageTypeEnum::CONTAINER), Some("create"), None)]
    #[case(None, Some("create"), None)]
    fn should_parse_network_action(
        #[case] typ: Option<EventMessageTypeEnum>,
        #[case] action: Option<&str>,
        #[case] expected: Option<NetworkEventStatus>,
    ) {
        let event = ContainerRuntimeEvent {
            inner: EventMessage {
                typ,
                action: action.map(str::to_string),
                ..Default::default()
            },
        };

        assert_eq!(event.network_action(), expected);
    }

    #[rstest]
    #[case(
        Some(EventMessageTypeEnum::VOLUME),
        Some("create"),
        Some(VolumeEventStatus::Create)
    )]
    #[case(
        Some(EventMessageTypeEnum::VOLUME),
        Some("mount"),
        Some(VolumeEventStatus::Mount)
    )]
    #[case(
        Some(EventMessageTypeEnum::VOLUME),
        Some("unmount"),
        Some(VolumeEventStatus::Unmount)
    )]
    #[case(
        Some(EventMessageTypeEnum::VOLUME),
        Some("destroy"),
        Some(VolumeEventStatus::Destroy)
    )]
    #[case(
        Some(EventMessageTypeEnum::VOLUME),
        Some("prune"),
        Some(VolumeEventStatus::Prune)
    )]
    #[case(Some(EventMessageTypeEnum::VOLUME), Some("unknown"), None)]
    #[case(Some(EventMessageTypeEnum::VOLUME), None, None)]
    #[case(Some(EventMessageTypeEnum::CONTAINER), Some("create"), None)]
    #[case(None, Some("create"), None)]
    fn should_parse_volume_action(
        #[case] typ: Option<EventMessageTypeEnum>,
        #[case] action: Option<&str>,
        #[case] expected: Option<VolumeEventStatus>,
    ) {
        let event = ContainerRuntimeEvent {
            inner: EventMessage {
                typ,
                action: action.map(str::to_string),
                ..Default::default()
            },
        };

        assert_eq!(event.volume_action(), expected);
    }
}
