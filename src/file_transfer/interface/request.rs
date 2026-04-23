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

use astarte_device_sdk::{FromEvent, event::FromEventError};
use tracing::{error, instrument, trace};

use crate::file_transfer::interface::{DeviceToServer, ServerToDevice};

use super::status::TransferDirection;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FileTransferRequest {
    Download(ServerToDevice),
    Upload(DeviceToServer),
}

impl FileTransferRequest {
    pub(crate) fn resp_id(&self) -> (&str, TransferDirection) {
        match self {
            FileTransferRequest::Download(req) => (&req.id, TransferDirection::Download),
            FileTransferRequest::Upload(req) => (&req.id, TransferDirection::Upload),
        }
    }
}

impl std::fmt::Display for FileTransferRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileTransferRequest::Download(server_to_device) => {
                write!(f, "Download(id: {})", server_to_device.id)
            }
            FileTransferRequest::Upload(device_to_server) => {
                write!(f, "Upload(id: {})", device_to_server.id)
            }
        }
    }
}

impl FromEvent for FileTransferRequest {
    type Err = FromEventError;

    #[instrument(fields(interface = event.interface, path = event.path), skip(event))]
    fn from_event(event: astarte_device_sdk::DeviceEvent) -> Result<Self, Self::Err> {
        trace!("parsing transfer event");

        match event.interface.as_str() {
            "io.edgehog.devicemanager.fileTransfer.ServerToDevice" => {
                ServerToDevice::from_event(event).map(FileTransferRequest::Download)
            }
            "io.edgehog.devicemanager.fileTransfer.DeviceToServer" => {
                DeviceToServer::from_event(event).map(FileTransferRequest::Upload)
            }
            _ => {
                error!("unrecognized event interface");

                Err(FromEventError::Interface(event.interface))
            }
        }
    }
}
