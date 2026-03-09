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

//! Transfer files from and to the Device

use tracing::{info, instrument, trace};

use crate::controller::actor::Actor;

use self::interface::FileTransferEvent;
use self::request::{DownloadReq, UploadReq};

pub(crate) mod interface;
mod request;

#[derive(Debug)]
pub(crate) struct FileTransfer {}

impl FileTransfer {
    pub fn new() -> Self {
        Self {}
    }
}

impl Actor for FileTransfer {
    type Msg = FileTransferEvent;

    fn task() -> &'static str {
        "file-transfer"
    }

    #[instrument(skip_all)]
    async fn init(&mut self) -> eyre::Result<()> {
        trace!("initializing file transfer");

        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle(&mut self, msg: Self::Msg) -> eyre::Result<()> {
        trace!("handle file transfer request");

        match msg {
            FileTransferEvent::Download(server_to_device) => {
                let _download = DownloadReq::try_from(server_to_device)?;

                info!("file download received");

                unimplemented!("file download")
            }
            FileTransferEvent::Upload(device_to_server) => {
                let _upload = UploadReq::try_from(device_to_server)?;

                info!("file upload received");

                unimplemented!("file upload")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use self::interface::tests::fs_server_to_device;
    use crate::file_transfer::interface::tests::fs_device_to_server;
    use crate::file_transfer::interface::{DeviceToServer, ServerToDevice};

    use super::*;

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "not implemented: file download")]
    async fn should_download_file(fs_server_to_device: ServerToDevice) {
        let mut transfer = FileTransfer::new();

        transfer
            .handle(FileTransferEvent::Download(fs_server_to_device))
            .await
            .unwrap();
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "not implemented: file upload")]
    async fn should_upload(fs_device_to_server: DeviceToServer) {
        let mut transfer = FileTransfer::new();

        transfer
            .handle(FileTransferEvent::Upload(fs_device_to_server))
            .await
            .unwrap();
    }
}
