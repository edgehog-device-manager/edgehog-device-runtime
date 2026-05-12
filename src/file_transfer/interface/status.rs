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

use astarte_device_sdk::AstarteData;
use astarte_device_sdk::{IntoAstarteObject, aggregate::AstarteObject};
use tracing::instrument;
use uuid::Uuid;

use crate::file_transfer::errno;
use crate::file_transfer::request::TransferJobTag;

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
pub(crate) struct FileTransferResponse {
    id: String,
    #[astarte_object(rename = "type")]
    ty: TransferDirection,
    code: i32,
    message: String,
}

impl FileTransferResponse {
    const INTERFACE: &str = "io.edgehog.devicemanager.fileTransfer.Response";

    pub(crate) fn success(transfer: FileTransferId) -> Self {
        Self {
            id: transfer.id.to_string(),
            ty: transfer.direction,
            code: errno::OK,
            message: String::new(),
        }
    }

    pub(crate) fn validation_error(
        id: &str,
        direction: TransferDirection,
        report: eyre::Report,
    ) -> Self {
        Self {
            id: id.to_string(),
            ty: direction,
            code: errno::to_errno(io::ErrorKind::InvalidInput),
            message: format!("{report:#}"),
        }
    }

    pub(crate) fn busy_error(id: &str, direction: TransferDirection) -> Self {
        Self {
            id: id.to_string(),
            ty: direction,
            code: errno::to_errno(io::ErrorKind::ResourceBusy),
            message: "file transfer request can't be handled currently".to_string(),
        }
    }

    pub(crate) fn runtime_error(
        id: Uuid,
        direction: TransferDirection,
        report: eyre::Report,
    ) -> Self {
        Self {
            id: id.to_string(),
            ty: direction,
            code: errno::to_errno(io::ErrorKind::Other),
            message: format!("{report:#}"),
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct FileTransferId {
    pub(crate) id: Uuid,
    pub(crate) direction: TransferDirection,
}

/// Direction of the transfer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransferDirection {
    /// Server to Device
    Download,
    /// Device to Server
    Upload,
}

impl std::fmt::Display for TransferDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferDirection::Download => f.write_str("server_to_device"),
            TransferDirection::Upload => f.write_str("device_to_server"),
        }
    }
}

impl From<TransferDirection> for AstarteData {
    fn from(value: TransferDirection) -> Self {
        AstarteData::String(value.to_string())
    }
}

impl From<TransferDirection> for TransferJobTag {
    fn from(value: TransferDirection) -> Self {
        match value {
            TransferDirection::Download => TransferJobTag::Download,
            TransferDirection::Upload => TransferJobTag::Upload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct FileTransferProgress {
    id: String,
    #[astarte_object(rename = "type")]
    ty: TransferDirection,
    bytes: i64,
    total_bytes: i64,
}

impl FileTransferProgress {
    const INTERFACE: &str = "io.edgehog.devicemanager.fileTransfer.Progress";

    pub(crate) fn start(
        transfer: FileTransferId,
        progress: bool,
        total_len: Option<u64>,
    ) -> Option<Self> {
        if !progress {
            return None;
        }

        total_len
            .map(|total| FileTransferProgress::with_total(transfer, 0, total))
            .or(Some(FileTransferProgress::create(transfer, 0)))
    }

    /// Create progress event with undefined total bytes, only positive value are accepted
    fn create(transfer: FileTransferId, bytes: u64) -> Self {
        Self {
            id: transfer.id.to_string(),
            ty: transfer.direction,
            bytes: super::to_i64(bytes),
            total_bytes: -1,
        }
    }

    /// Create progress event with total bytes, only positive value are accepted
    fn with_total(transfer: FileTransferId, bytes: u64, total_bytes: u64) -> Self {
        Self {
            id: transfer.id.to_string(),
            ty: transfer.direction,
            bytes: super::to_i64(bytes),
            total_bytes: super::to_i64(total_bytes),
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

    pub(crate) fn set_bytes(&mut self, bytes: u64) {
        self.bytes = super::to_i64(bytes);
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::with_insta;

    use rstest::{Context, rstest};

    use super::*;

    fn mk_resp_busy() -> FileTransferResponse {
        FileTransferResponse::busy_error(
            "3473aac8-42d8-4e4a-942f-609df942d6b4",
            TransferDirection::Download,
        )
    }

    fn mk_resp_validation() -> FileTransferResponse {
        FileTransferResponse::validation_error(
            "3473aac8-42d8-4e4a-942f-609df942d6b4",
            TransferDirection::Download,
            eyre::eyre!("validation error encountered"),
        )
    }

    fn mk_resp_ok() -> FileTransferResponse {
        FileTransferResponse::success(FileTransferId {
            id: "3473aac8-42d8-4e4a-942f-609df942d6b4".parse().unwrap(),
            direction: TransferDirection::Download,
        })
    }

    #[rstest]
    #[case(mk_resp_ok())]
    #[case(mk_resp_busy())]
    #[case(mk_resp_validation())]
    fn serialize_response(#[context] ctx: Context, #[case] resp: FileTransferResponse) {
        let data = AstarteObject::try_from(resp).unwrap();

        let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

        with_insta!({
            insta::assert_debug_snapshot!(name, data);
        });
    }

    fn mk_progress_no_tot() -> FileTransferProgress {
        let id = FileTransferId {
            id: "3473aac8-42d8-4e4a-942f-609df942d6b4".parse().unwrap(),
            direction: TransferDirection::Download,
        };

        FileTransferProgress::create(id, 1 << 10)
    }

    fn mk_progress_tot() -> FileTransferProgress {
        let id = FileTransferId {
            id: "3473aac8-42d8-4e4a-942f-609df942d6b4".parse().unwrap(),
            direction: TransferDirection::Download,
        };

        FileTransferProgress::with_total(id, 1 << 10, 1 << 12)
    }

    #[rstest]
    #[case(mk_progress_no_tot())]
    #[case(mk_progress_tot())]
    fn serialize_progress(#[context] ctx: Context, #[case] progress: FileTransferProgress) {
        let data = AstarteObject::try_from(progress).unwrap();

        let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

        with_insta!({
            insta::assert_debug_snapshot!(name, data);
        });
    }
}
