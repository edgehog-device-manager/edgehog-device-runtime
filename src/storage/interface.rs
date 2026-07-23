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

use std::io;

use astarte_device_sdk::{AstarteData, FromEvent, IntoAstarteObject, aggregate::AstarteObject};
use eyre::eyre;
use tracing::instrument;

use crate::{
    file_transfer::errno,
    storage::{StorageJobTag, request::FileStorageId},
};

#[derive(Debug, Clone, PartialEq, FromEvent, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.storage.DeleteFile",
    aggregation = "object",
    path = "/request",
    rename_all = "camelCase"
)]
#[astarte_object(rename_all = "camelCase")]
pub struct DeleteFile {
    pub(crate) id: String,
    pub(crate) file_id: String,
    pub(crate) force: bool,
}

impl DeleteFile {
    pub(crate) const RESPONSE_TYPE: ActionType = ActionType::Delete;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ActionType {
    Delete,
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Delete => f.write_str("delete"),
        }
    }
}

impl From<ActionType> for AstarteData {
    fn from(value: ActionType) -> Self {
        AstarteData::String(value.to_string())
    }
}

impl TryFrom<StorageJobTag> for ActionType {
    type Error = eyre::Error;

    fn try_from(value: StorageJobTag) -> Result<Self, Self::Error> {
        match value {
            StorageJobTag::CleanUp => Err(eyre!("cleanup job shouldn't send reseponses")),
            StorageJobTag::Delete => Ok(ActionType::Delete),
        }
    }
}

impl From<ActionType> for StorageJobTag {
    fn from(value: ActionType) -> Self {
        match value {
            ActionType::Delete => StorageJobTag::Delete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
pub(crate) struct FileStorageResponse {
    id: String,
    #[astarte_object(rename = "type")]
    ty: ActionType,
    code: i32,
    messages: Vec<String>,
}

impl FileStorageResponse {
    const INTERFACE: &str = "io.edgehog.devicemanager.storage.Response";

    pub(crate) fn success(store: FileStorageId) -> Self {
        Self {
            id: store.id.to_string(),
            ty: store.ty,
            code: errno::OK,
            messages: Vec::new(),
        }
    }

    fn to_backtrace(report: eyre::Report) -> Vec<String> {
        let mut backtrace = vec![report.to_string()];

        backtrace.extend(report.chain().map(|e| e.to_string()));

        backtrace
    }

    pub(crate) fn validation_error(id: &str, ty: ActionType, report: eyre::Report) -> Self {
        Self {
            id: id.to_string(),
            ty,
            code: errno::to_errno(io::ErrorKind::InvalidInput),
            messages: Self::to_backtrace(report),
        }
    }

    pub(crate) fn busy_error(id: &str, ty: ActionType) -> Self {
        Self {
            id: id.to_string(),
            ty,
            code: errno::to_errno(io::ErrorKind::ResourceBusy),
            messages: vec!["file transfer request can't be handled currently".to_string()],
        }
    }

    pub(crate) fn runtime_error(store: FileStorageId, report: eyre::Report) -> Self {
        Self {
            id: store.id.to_string(),
            ty: store.ty,
            code: errno::to_errno(io::ErrorKind::Other),
            messages: Self::to_backtrace(report),
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn send<C>(self, device: &mut C) -> eyre::Result<()>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        device
            .send_object(Self::INTERFACE, "/request", AstarteObject::try_from(self)?)
            .await
            .map_err(eyre::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{DeviceEvent, chrono::Utc};
    use rstest::{Context, fixture, rstest};
    use uuid::Uuid;

    use crate::tests::with_insta;

    use super::*;

    #[fixture]
    fn delete_event() -> AstarteObject {
        AstarteObject::from_iter(vec![
            (
                "id".to_string(),
                AstarteData::from(Uuid::from_u128(0u128).to_string()),
            ),
            (
                "fileId".to_string(),
                AstarteData::from(Uuid::new_v4().to_string()),
            ),
            ("force".to_string(), AstarteData::Boolean(true)),
        ])
    }

    #[rstest]
    fn delete_file_deserialize(delete_event: AstarteObject) {
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.storage.DeleteFile".to_string(),
            path: "/request".to_string(),
            data: astarte_device_sdk::Value::Object {
                data: delete_event,
                timestamp: Utc::now(),
            },
        };

        DeleteFile::from_event(event).unwrap();
    }

    #[fixture]
    fn storage_response_ok() -> FileStorageResponse {
        FileStorageResponse::success(FileStorageId {
            id: Uuid::from_u128(0u128),
            ty: ActionType::Delete,
        })
    }

    #[fixture]
    fn storage_response_validation() -> FileStorageResponse {
        FileStorageResponse::validation_error(
            &Uuid::from_u128(0u128).to_string(),
            ActionType::Delete,
            eyre::eyre!("test error"),
        )
    }

    #[fixture]
    fn storage_response_busy() -> FileStorageResponse {
        FileStorageResponse::busy_error(&Uuid::from_u128(0u128).to_string(), ActionType::Delete)
    }

    #[fixture]
    fn storage_response_runtime() -> FileStorageResponse {
        FileStorageResponse::runtime_error(
            FileStorageId {
                id: Uuid::from_u128(0u128),
                ty: ActionType::Delete,
            },
            eyre::eyre!("runtime test error"),
        )
    }

    #[rstest]
    #[case(storage_response_ok())]
    #[case(storage_response_validation())]
    #[case(storage_response_busy())]
    #[case(storage_response_runtime())]
    fn storage_response_into_object(
        #[case] response: FileStorageResponse,
        #[context] ctx: Context,
    ) {
        let obj = AstarteObject::try_from(response).unwrap();

        let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

        with_insta!({
            insta::assert_debug_snapshot!(name, obj);
        });
    }
}
