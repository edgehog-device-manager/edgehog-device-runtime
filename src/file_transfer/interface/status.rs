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
use tracing::{instrument, warn};
use uuid::Uuid;

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
            code: 0,
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
            code: to_errno(io::ErrorKind::InvalidInput),
            message: format!("{report:#}"),
        }
    }

    pub(crate) fn busy_error(id: &str, direction: TransferDirection) -> Self {
        Self {
            id: id.to_string(),
            ty: direction,
            code: to_errno(io::ErrorKind::ResourceBusy),
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
            code: to_errno(io::ErrorKind::Other),
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

// TODO use error number defined in libc or a wrapper library. Also check the correctness of the errors
fn to_errno(kind: io::ErrorKind) -> i32 {
    match kind {
        io::ErrorKind::NotFound => 2,                 // ENOENT
        io::ErrorKind::PermissionDenied => 13,        // EACCES
        io::ErrorKind::ConnectionRefused => 111,      // ECONNREFUSED
        io::ErrorKind::ConnectionReset => 104,        // ECONNRESET
        io::ErrorKind::HostUnreachable => 113,        // EHOSTUNREACH
        io::ErrorKind::NetworkUnreachable => 101,     // ENETUNREACH
        io::ErrorKind::ConnectionAborted => 103,      // ECONNABORTED
        io::ErrorKind::NotConnected => 107,           // ENOTCONN
        io::ErrorKind::AddrInUse => 98,               // EADDRINUSE
        io::ErrorKind::AddrNotAvailable => 99,        // EADDRNOTAVAIL
        io::ErrorKind::NetworkDown => 100,            // ENETDOWN
        io::ErrorKind::BrokenPipe => 32,              // EPIPE
        io::ErrorKind::AlreadyExists => 17,           // EEXIST
        io::ErrorKind::WouldBlock => 11,              // EWOULDBLOCK / EAGAIN
        io::ErrorKind::NotADirectory => 20,           // ENOTDIR
        io::ErrorKind::IsADirectory => 21,            // EISDIR
        io::ErrorKind::DirectoryNotEmpty => 39,       // ENOTEMPTY
        io::ErrorKind::ReadOnlyFilesystem => 30,      // EROFS
        io::ErrorKind::StaleNetworkFileHandle => 116, // ESTALE
        io::ErrorKind::InvalidInput => 22,            // EINVAL
        io::ErrorKind::InvalidData => 22,             // EINVAL (Rust specific, closest match)
        io::ErrorKind::TimedOut => 110,               // ETIMEDOUT
        io::ErrorKind::WriteZero => 28,               // ENOSPC (Closest approximation)
        io::ErrorKind::StorageFull => 28,             // ENOSPC
        io::ErrorKind::NotSeekable => 29,             // ESPIPE
        io::ErrorKind::QuotaExceeded => 122,          // EDQUOT
        io::ErrorKind::FileTooLarge => 27,            // EFBIG
        io::ErrorKind::ResourceBusy => 16,            // EBUSY
        io::ErrorKind::ExecutableFileBusy => 26,      // ETXTBSY
        io::ErrorKind::Deadlock => 35,                // EDEADLK
        io::ErrorKind::CrossesDevices => 18,          // EXDEV
        io::ErrorKind::TooManyLinks => 31,            // EMLINK
        io::ErrorKind::InvalidFilename => 36,         // ENAMETOOLONG
        io::ErrorKind::ArgumentListTooLong => 7,      // E2BIG
        io::ErrorKind::Interrupted => 4,              // EINTR
        io::ErrorKind::Unsupported => 95,             // ENOTSUP / EOPNOTSUPP
        io::ErrorKind::UnexpectedEof => 61,           // ENODATA (Closest approximation)
        io::ErrorKind::OutOfMemory => 12,             // ENOMEM
        io::ErrorKind::Other => 22,                   // EINVAL (Generic fallback)
        _ => 22,                                      // EINVAL (Generic fallback)
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

    fn to_i64(unsigned: u64) -> i64 {
        i64::try_from(unsigned)
            .inspect_err(|error| warn!(%error, "progress bytes overflow"))
            .unwrap_or(i64::MAX)
    }

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
            bytes: Self::to_i64(bytes),
            total_bytes: -1,
        }
    }

    /// Create progress event with total bytes, only positive value are accepted
    fn with_total(transfer: FileTransferId, bytes: u64, total_bytes: u64) -> Self {
        Self {
            id: transfer.id.to_string(),
            ty: transfer.direction,
            bytes: Self::to_i64(bytes),
            total_bytes: Self::to_i64(total_bytes),
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
        self.bytes = Self::to_i64(bytes);
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
