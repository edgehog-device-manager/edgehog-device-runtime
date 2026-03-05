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

use std::path::PathBuf;

use tracing::{info, instrument};

use crate::controller::actor::Actor;

use self::file_system::FileOptions;
use self::file_system::store::{FileStorage, Fs, Limits};
use self::file_system::stream::{Pipe, Streaming, SysPipe};
use self::interface::FileTransferEvent;
use self::request::{DownloadReq, Target, UploadReq};

mod file_system;
pub(crate) mod interface;
mod request;

#[derive(Debug)]
pub(crate) struct FileTransfer<F, S> {
    storage: FileStorage<F>,
    stream: Streaming<S>,
}

impl FileTransfer<Fs, SysPipe> {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            storage: FileStorage::new(dir),
            stream: Streaming::with_sys(),
        }
    }
}

impl<F, S> Actor for FileTransfer<F, S>
where
    F: Limits + Send + Sync,
    S: Pipe + Send + Sync,
{
    type Msg = FileTransferEvent;

    fn task() -> &'static str {
        "file-transfer"
    }

    #[instrument(skip_all)]
    async fn init(&mut self) -> eyre::Result<()> {
        Ok(())
    }

    #[instrument(skip(self))]
    async fn handle(&mut self, msg: Self::Msg) -> eyre::Result<()> {
        match msg {
            FileTransferEvent::Download(server_to_device) => {
                let download = DownloadReq::try_from(server_to_device)?;

                info!("file download received");

                match download.destination {
                    Target::Storage => {
                        if self.storage.file_exists(&download.id).await? {
                            info!("file already exists");

                            return Ok(());
                        }

                        let opt = FileOptions::from(download);

                        let (_file, _digest) = self.storage.create_write_handle(&opt).await?;

                        // TODO: download...
                        unimplemented!("file download");

                        #[expect(unreachable_code)]
                        self.storage.finalize_write(_file, &opt).await?;
                    }
                    Target::Stream => {
                        let opt = FileOptions::from(download);

                        let (_file, _digest) = self.stream.open_writer(&opt).await?;

                        unimplemented!("file download");
                    }
                }
            }
            FileTransferEvent::Upload(device_to_server) => {
                let upload = UploadReq::try_from(device_to_server)?;

                info!("file upload received");

                match upload.source {
                    Target::Storage => {
                        let _file = self.storage.open_read(&upload.id).await?;

                        unimplemented!("file upload")
                    }
                    Target::Stream => {
                        let _file = self.stream.create_reader(&upload.id).await?;

                        unimplemented!("file download");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use tempdir::TempDir;

    use self::interface::tests::fs_server_to_device;
    use crate::file_transfer::interface::tests::fs_device_to_server;
    use crate::file_transfer::interface::{DeviceToServer, ServerToDevice};

    use super::*;

    fn mk_transfe(prefix: &str) -> (FileTransfer<Fs, SysPipe>, TempDir) {
        let dir = TempDir::new(prefix).unwrap();

        (FileTransfer::new(dir.path().to_path_buf()), dir)
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "not implemented: file download")]
    async fn should_download(fs_server_to_device: ServerToDevice) {
        let (mut transfer, _dir) = mk_transfe("download");

        transfer
            .handle(FileTransferEvent::Download(fs_server_to_device))
            .await
            .unwrap();
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "not implemented: file upload")]
    async fn should_upload(fs_device_to_server: DeviceToServer) {
        let (mut transfer, dir) = mk_transfe("upload");

        tokio::fs::write(dir.path().join(&fs_device_to_server.id), "Hello world!")
            .await
            .unwrap();

        transfer
            .handle(FileTransferEvent::Upload(fs_device_to_server))
            .await
            .unwrap();
    }
}
