// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

use std::fmt::Display;

use astarte_device_sdk::event::FromEventError;
use astarte_device_sdk::{FromEvent, IntoAstarteObject};
use tracing::{error, instrument};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FileTransferEvent {
    Download(ServerToDevice),
    Upload(DeviceToServer),
}

impl Display for FileTransferEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileTransferEvent::Download(server_to_device) => {
                write!(f, "Download(id: {})", server_to_device.id)
            }
            FileTransferEvent::Upload(device_to_server) => {
                write!(f, "Upload(id: {})", device_to_server.id)
            }
        }
    }
}

impl FromEvent for FileTransferEvent {
    type Err = astarte_device_sdk::event::FromEventError;

    #[instrument(fields(interface = event.interface, path = event.path), skip(event))]
    fn from_event(event: astarte_device_sdk::DeviceEvent) -> Result<Self, Self::Err> {
        match event.interface.as_str() {
            "io.edgehog.devicemanager.fileTransfer.posix.ServerToDevice" => {
                ServerToDevice::from_event(event).map(FileTransferEvent::Download)
            }
            "io.edgehog.devicemanager.fileTransfer.posix.DeviceToServer" => {
                DeviceToServer::from_event(event).map(FileTransferEvent::Upload)
            }
            _ => {
                error!("unrecognized event interface");

                Err(FromEventError::Interface(event.interface))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, FromEvent, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.fileTransfer.posix.ServerToDevice",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct ServerToDevice {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) http_header_key: Vec<String>,
    pub(crate) http_header_value: Vec<String>,
    pub(crate) compression: String,
    pub(crate) file_size_bytes: i64,
    pub(crate) progress: bool,
    pub(crate) digest: String,
    pub(crate) ttl_seconds: i64,
    pub(crate) file_mode: i64,
    pub(crate) user_id: i64,
    pub(crate) group_id: i64,
    pub(crate) destination_type: String,
    pub(crate) destination: String,
}

#[derive(Debug, Clone, PartialEq, FromEvent, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.fileTransfer.posix.DeviceToServer",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct DeviceToServer {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) http_header_key: Vec<String>,
    pub(crate) http_header_value: Vec<String>,
    pub(crate) compression: String,
    pub(crate) progress: bool,
    pub(crate) source_type: String,
    pub(crate) source: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::DeviceEvent;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use rstest::{fixture, rstest};

    use crate::tests::with_insta;

    use super::*;

    impl FileTransferEvent {
        pub(crate) fn try_into_download(self) -> Option<ServerToDevice> {
            if let Self::Download(v) = self {
                Some(v)
            } else {
                None
            }
        }

        pub(crate) fn try_into_upload(self) -> Option<DeviceToServer> {
            if let Self::Upload(v) = self {
                Some(v)
            } else {
                None
            }
        }
    }

    #[fixture]
    pub(crate) fn fs_server_to_device() -> ServerToDevice {
        ServerToDevice {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url: "https://s3.example.com".to_string(),
            http_header_key: vec!["authorization".to_string()],
            http_header_value: vec!["Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()],
            compression: "tar.gz".to_string(),
            file_size_bytes: 4096,
            progress: true,
            digest: "sha256:28babb1cdf8aea6b62acc1097fdc83482cbf6e11c4fe7dcb39ae1682776baec5".to_string(),
            ttl_seconds: 0,
            file_mode: 544,
            user_id: 1000,
            group_id: 100,
            destination_type: "storage".to_string(),
            destination: String::new(),
        }
    }

    #[fixture]
    pub(crate) fn fs_device_to_server() -> DeviceToServer {
        DeviceToServer {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url: "https://s3.example.com".to_string(),
            http_header_key: vec!["authorization".to_string()],
            http_header_value: vec!["Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()],
            compression: "tar.gz".to_string(),
            progress: true,
            source_type: "storage".to_string(),
            source: String::new(),
        }
    }

    #[rstest]
    fn from_and_to_event_download(fs_server_to_device: ServerToDevice) {
        let data = AstarteObject::try_from(fs_server_to_device.clone()).unwrap();

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.fileTransfer.posix.ServerToDevice".to_string(),
            path: "/request".to_string(),
            data: astarte_device_sdk::Value::Object {
                data: data.clone(),
                timestamp: Utc::now(),
            },
        };

        let interface = FileTransferEvent::from_event(event).unwrap();

        let download = interface.try_into_download().unwrap();

        assert_eq!(download, fs_server_to_device);

        with_insta!({
            insta::assert_debug_snapshot!(data);
        });
    }

    #[rstest]
    fn from_and_to_event_upload(fs_device_to_server: DeviceToServer) {
        let data = AstarteObject::try_from(fs_device_to_server.clone()).unwrap();

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.fileTransfer.posix.DeviceToServer".to_string(),
            path: "/request".to_string(),
            data: astarte_device_sdk::Value::Object {
                data: data.clone(),
                timestamp: Utc::now(),
            },
        };

        let interface = FileTransferEvent::from_event(event).unwrap();

        let upload = interface.try_into_upload().unwrap();

        assert_eq!(upload, fs_device_to_server);

        with_insta!({
            insta::assert_debug_snapshot!(data);
        });
    }
}
