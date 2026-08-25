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

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::{AstarteData, FromEvent};
use tracing::warn;

pub(crate) mod capabilities;
pub(crate) mod file;
pub(crate) mod request;
pub(crate) mod status;

#[derive(Debug, Clone, PartialEq, FromEvent)]
#[from_event(
    interface = "io.edgehog.devicemanager.fileTransfer.ServerToDevice",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
pub(crate) struct ServerToDevice {
    #[mapping(required)]
    pub(crate) id: String,
    #[mapping(required)]
    pub(crate) url: String,
    #[mapping(required)]
    pub(crate) encoding: String,
    #[mapping(required)]
    pub(crate) file_size_bytes: i64,
    #[mapping(required)]
    pub(crate) digest: String,
    #[mapping(required)]
    pub(crate) destination_type: String,
    #[mapping(required)]
    pub(crate) destination: String,
    pub(crate) http_header_keys: Option<Vec<String>>,
    pub(crate) http_header_values: Option<Vec<String>>,
    pub(crate) progress: Option<bool>,
    pub(crate) ttl_seconds: Option<i64>,
    pub(crate) file_mode: Option<i64>,
    pub(crate) user_id: Option<i64>,
    pub(crate) group_id: Option<i64>,
}

impl From<ServerToDevice> for AstarteObject {
    fn from(value: ServerToDevice) -> Self {
        let mut data = AstarteObject::from_iter(
            [
                ("id", AstarteData::from(value.id)),
                ("url", AstarteData::from(value.url)),
                ("encoding", AstarteData::from(value.encoding)),
                ("fileSizeBytes", AstarteData::from(value.file_size_bytes)),
                ("digest", AstarteData::from(value.digest)),
                ("destinationType", AstarteData::from(value.destination_type)),
                ("destination", AstarteData::from(value.destination)),
            ]
            .map(|(k, v)| (k.to_string(), v)),
        );

        if let Some(value) = value.http_header_keys {
            data.insert("httpHeaderKeys".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.http_header_values {
            data.insert("httpHeaderValues".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.progress {
            data.insert("progress".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.ttl_seconds {
            data.insert("ttlSeconds".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.file_mode {
            data.insert("fileMode".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.user_id {
            data.insert("userId".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.group_id {
            data.insert("groupId".to_string(), AstarteData::from(value));
        }

        data
    }
}

#[derive(Debug, Clone, PartialEq, FromEvent)]
#[from_event(
    interface = "io.edgehog.devicemanager.fileTransfer.DeviceToServer",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
pub(crate) struct DeviceToServer {
    #[mapping(required)]
    pub(crate) id: String,
    #[mapping(required)]
    pub(crate) url: String,
    #[mapping(required)]
    pub(crate) encoding: String,
    #[mapping(required)]
    pub(crate) source_type: String,
    #[mapping(required)]
    pub(crate) source: String,
    pub(crate) http_header_keys: Option<Vec<String>>,
    pub(crate) http_header_values: Option<Vec<String>>,
    pub(crate) progress: Option<bool>,
}

impl From<DeviceToServer> for AstarteObject {
    fn from(value: DeviceToServer) -> Self {
        let mut data = AstarteObject::from_iter(
            [
                ("id", AstarteData::from(value.id)),
                ("url", AstarteData::from(value.url)),
                ("encoding", AstarteData::from(value.encoding)),
                ("sourceType", AstarteData::from(value.source_type)),
                ("source", AstarteData::from(value.source)),
            ]
            .map(|(k, v)| (k.to_string(), v)),
        );

        if let Some(value) = value.http_header_keys {
            data.insert("httpHeaderKeys".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.http_header_values {
            data.insert("httpHeaderValues".to_string(), AstarteData::from(value));
        }
        if let Some(value) = value.progress {
            data.insert("progress".to_string(), AstarteData::from(value));
        }

        data
    }
}

fn to_i64(unsigned: u64) -> i64 {
    i64::try_from(unsigned)
        .inspect_err(|error| warn!(%error, "progress bytes overflow"))
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::DeviceEvent;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use rstest::{fixture, rstest};

    use crate::file_transfer::interface::request::FileTransferRequest;
    use crate::tests::with_insta;

    use super::*;

    impl FileTransferRequest {
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
            http_header_keys: Some(vec!["authorization".to_string()]),
            http_header_values: Some(vec!["Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()]),
            encoding: "tar.gz".to_string(),
            file_size_bytes: 4096,
            progress: Some(true),
            digest: "sha256:28babb1cdf8aea6b62acc1097fdc83482cbf6e11c4fe7dcb39ae1682776baec5".to_string(),
            ttl_seconds: Some(0),
            file_mode: Some(544),
            user_id: Some(1000),
            group_id: Some(100),
            destination_type: "storage".to_string(),
            destination: String::new(),
        }
    }

    #[fixture]
    pub(crate) fn fs_device_to_server() -> DeviceToServer {
        DeviceToServer {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url: "https://s3.example.com".to_string(),
            http_header_keys: Some(vec!["authorization".to_string()]),
            http_header_values: Some(vec!["Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()]),
            encoding: "tar.gz".to_string(),
            progress: Some(true),
            source_type: "storage".to_string(),
            source: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
        }
    }

    #[rstest]
    fn from_and_to_event_download(fs_server_to_device: ServerToDevice) {
        let data = AstarteObject::from(fs_server_to_device.clone());

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.fileTransfer.ServerToDevice".to_string(),
            path: "/request".to_string(),
            data: astarte_device_sdk::Value::Object {
                data: data.clone(),
                timestamp: Utc::now(),
            },
        };

        let interface = FileTransferRequest::from_event(event).unwrap();

        let download = interface.try_into_download().unwrap();

        assert_eq!(download, fs_server_to_device);

        with_insta!({
            insta::assert_debug_snapshot!(data);
        });
    }

    #[rstest]
    fn from_and_to_event_upload(fs_device_to_server: DeviceToServer) {
        let data = AstarteObject::from(fs_device_to_server.clone());

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.fileTransfer.DeviceToServer".to_string(),
            path: "/request".to_string(),
            data: astarte_device_sdk::Value::Object {
                data: data.clone(),
                timestamp: Utc::now(),
            },
        };

        let interface = FileTransferRequest::from_event(event).unwrap();

        let upload = interface.try_into_upload().unwrap();

        assert_eq!(upload, fs_device_to_server);

        with_insta!({
            insta::assert_debug_snapshot!(data);
        });
    }
}
