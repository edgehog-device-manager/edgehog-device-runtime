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

use std::io::SeekFrom;
use std::path::PathBuf;

use aws_lc_rs::digest;
use eyre::{Context, OptionExt};
use tokio::io::{AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tracing::{info, instrument};

use crate::controller::actor::Actor;
use crate::http::{DownloadRange, FileDownloadResponse, FileTransferHttpClient};

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
    client: FileTransferHttpClient,
}

impl FileTransfer<Fs, SysPipe> {
    pub fn new(dir: PathBuf) -> eyre::Result<Self> {
        Ok(Self {
            storage: FileStorage::new(dir),
            stream: Streaming::new(),
            client: FileTransferHttpClient::create()
                .wrap_err("can't construct file transfer client")?,
        })
    }
}

impl<F, S> FileTransfer<F, S> {
    async fn write_chunks<W>(
        sink: &mut W,
        digest: &mut digest::Context,
        mut download_resp: FileDownloadResponse,
    ) -> eyre::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        while let Some(bytes) = download_resp
            .chunk()
            .await
            .wrap_err("error while getting response chunk")?
        {
            sink.write_all(&bytes).await?;
            sink.flush().await?;

            digest.update(&bytes);
        }

        Ok(())
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

                        let opt = FileOptions::from(&download);

                        let (mut file, mut digest) = self.storage.create_write_handle(&opt).await?;

                        let download_range =
                            DownloadRange::create(file.current_size(), opt.file_size)
                                .ok_or_eyre("invalid download range")?;

                        let file_resp = self
                            .client
                            .download(&download.url, download.headers, download_range)
                            .await
                            .wrap_err("download request error")?;

                        if file_resp.range().start() != file.current_size() {
                            file.seek(SeekFrom::Start(file_resp.range().start()))
                                .await?;
                        }

                        Self::write_chunks(&mut file, &mut digest, file_resp).await?;

                        self.storage.finalize_write(file, &opt).await
                    }
                    Target::Stream => {
                        let opt = FileOptions::from(&download);

                        let (mut file, mut digest) = self.stream.open_writer(&opt).await?;

                        let range = DownloadRange::with_size(opt.file_size);

                        let file_resp = self
                            .client
                            .download(&download.url, download.headers, range)
                            .await
                            .wrap_err("download request error")?;

                        Self::write_chunks(&mut file, &mut digest, file_resp).await?;

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
    use httpmock::Mock;
    use httpmock::{Method::GET, MockServer};
    use rstest::{fixture, rstest};
    use tempdir::TempDir;

    use crate::file_transfer::interface::tests::fs_device_to_server;
    use crate::file_transfer::interface::{DeviceToServer, ServerToDevice};

    use super::*;

    fn mk_transfer(prefix: &str) -> (FileTransfer<Fs, SysPipe>, TempDir) {
        let dir = TempDir::new(prefix).unwrap();

        (FileTransfer::new(dir.path().to_path_buf()).unwrap(), dir)
    }

    #[fixture]
    #[once]
    fn mock_fs() -> MockServer {
        MockServer::start()
    }

    struct MockedFsCall<'a> {
        call_get: Mock<'a>,
        call_full_get: Option<Mock<'a>>,
        event: ServerToDevice,
        content: Vec<u8>,
    }

    impl<'a> MockedFsCall<'a> {
        fn new(call_get: Mock<'a>, event: ServerToDevice, content: Vec<u8>) -> Self {
            Self {
                call_get,
                event,
                content,
                call_full_get: None,
            }
        }

        fn with_get_full(
            call_get: Mock<'a>,
            call_full_get: Mock<'a>,
            event: ServerToDevice,
            content: Vec<u8>,
        ) -> Self {
            Self {
                call_get,
                event,
                content,
                call_full_get: Some(call_full_get),
            }
        }
    }

    #[fixture]
    async fn mock_fs_download<'a>(mock_fs: &'a MockServer) -> MockedFsCall<'a> {
        let url = mock_fs.url("/file");
        let size = 1024usize;
        let content = vec![1u8; size];
        let mut digest = digest::Context::new(&digest::SHA256);
        digest.update(&content);
        let digest = format!(
            "sha256:{}",
            digest
                .finish()
                .as_ref()
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<String>()
        );

        let headers = [
            ("authorization".to_string(), "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk811kA==".to_string()),
        ];

        let call_get = mock_fs
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file")
                    .header_includes("range", "bytes=0-");

                headers
                    .iter()
                    .fold(when, |when, (n, s)| when.header_includes(n, s));

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 0-1023/1024")
                    .body(&content);
            })
            .await;

        let event = ServerToDevice {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url,
            http_header_key: headers.iter().map(|(k, _)| k).cloned().collect(),
            http_header_value: headers.iter().map(|(_, v)| v).cloned().collect(),
            compression: "tar.gz".to_string(),
            file_size_bytes: i64::try_from(size).unwrap(),
            progress: true,
            digest,
            ttl_seconds: 0,
            file_mode: 544,
            user_id: 1000,
            group_id: 100,
            destination: "storage".to_string(),
        };

        MockedFsCall::new(call_get, event, content)
    }

    #[fixture]
    async fn mock_fs_download_part<'a>(mock_fs: &'a MockServer) -> MockedFsCall<'a> {
        let url = mock_fs.url("/file-part");
        let size = 1024usize;
        let content = vec![3u8; size];
        let mut digest = digest::Context::new(&digest::SHA256);
        digest.update(&content);
        let digest = format!(
            "sha256:{}",
            digest
                .finish()
                .as_ref()
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<String>()
        );
        let partial_content = vec![3u8; 512];

        let headers = [
            ("authorization".to_string(), "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()),
        ];

        let call_get = mock_fs
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part")
                    .header_includes("range", format!("bytes={}-", partial_content.len()));

                headers
                    .iter()
                    .fold(when, |when, (n, s)| when.header_includes(n, s));

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 512-1023/1024")
                    .body(&partial_content);
            })
            .await;
        let event = ServerToDevice {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url,
            http_header_key: headers.iter().map(|(k, _)| k).cloned().collect(),
            http_header_value: headers.iter().map(|(_, v)| v).cloned().collect(),
            compression: "tar.gz".to_string(),
            file_size_bytes: i64::try_from(size).unwrap(),
            progress: true,
            digest,
            ttl_seconds: 0,
            file_mode: 544,
            user_id: 1000,
            group_id: 100,
            destination: "storage".to_string(),
        };

        MockedFsCall::new(call_get, event, content)
    }

    #[fixture]
    async fn mock_fs_download_part_not_satisfied<'a>(mock_fs: &'a MockServer) -> MockedFsCall<'a> {
        let url = mock_fs.url("/file-part-not-206");
        let size = 1024usize;
        let content = vec![4u8; size];
        let mut digest = digest::Context::new(&digest::SHA256);
        digest.update(&content);
        let digest = format!(
            "sha256:{}",
            digest
                .finish()
                .as_ref()
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<String>()
        );

        let headers = [
            ("authorization".to_string(), "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==".to_string()),
        ];

        let call_get = mock_fs
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part-not-206")
                    .header_includes("range", "bytes=512-");

                headers
                    .iter()
                    .fold(when, |when, (n, s)| when.header_includes(n, s));

                then.status(reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
            })
            .await;
        let call_get_full = mock_fs
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part-not-206")
                    .header_missing("range");

                headers
                    .iter()
                    .fold(when, |when, (n, s)| when.header_includes(n, s));

                then.status(reqwest::StatusCode::OK)
                    .header("Content-Length", content.len().to_string())
                    .body(&content);
            })
            .await;

        let event = ServerToDevice {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".to_string(),
            url,
            http_header_key: headers.iter().map(|(k, _)| k).cloned().collect(),
            http_header_value: headers.iter().map(|(_, v)| v).cloned().collect(),
            compression: "tar.gz".to_string(),
            file_size_bytes: i64::try_from(size).unwrap(),
            progress: true,
            digest,
            ttl_seconds: 0,
            file_mode: 544,
            user_id: 1000,
            group_id: 100,
            destination: "storage".to_string(),
        };

        MockedFsCall::with_get_full(call_get, call_get_full, event, content)
    }

    #[rstest]
    #[tokio::test]
    async fn should_download(#[future] mock_fs_download: MockedFsCall<'_>) {
        let mock_download_event = mock_fs_download.await;

        let (mut transfer, dir) = mk_transfer("should_download");
        let complete_file = dir.path().join(&mock_download_event.event.id);

        transfer
            .handle(FileTransferEvent::Download(mock_download_event.event))
            .await
            .unwrap();

        mock_download_event.call_get.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_part(#[future] mock_fs_download_part: MockedFsCall<'_>) {
        let mock_download_event = mock_fs_download_part.await;

        let (mut transfer, dir) = mk_transfer("download");
        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.event.id));
        let complete_file = dir.path().join(&mock_download_event.event.id);

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer
            .handle(FileTransferEvent::Download(mock_download_event.event))
            .await
            .unwrap();

        mock_download_event.call_get.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_full_when_unsatisfiable(
        #[future] mock_fs_download_part_not_satisfied: MockedFsCall<'_>,
    ) {
        let mock_download_event = mock_fs_download_part_not_satisfied.await;
        let dir = TempDir::new("download").unwrap();

        let mut transfer = FileTransfer::new(dir.path().to_path_buf()).unwrap();
        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.event.id));
        let complete_file = dir.path().join(&mock_download_event.event.id);

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer
            .handle(FileTransferEvent::Download(mock_download_event.event))
            .await
            .unwrap();

        mock_download_event.call_get.assert_async().await;
        mock_download_event
            .call_full_get
            .unwrap()
            .assert_async()
            .await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    #[should_panic(expected = "not implemented: file upload")]
    async fn should_upload(fs_device_to_server: DeviceToServer) {
        let (mut transfer, dir) = mk_transfer("upload");
        tokio::fs::write(dir.path().join(&fs_device_to_server.id), "Hello world!")
            .await
            .unwrap();

        transfer
            .handle(FileTransferEvent::Upload(fs_device_to_server))
            .await
            .unwrap();
    }
}
