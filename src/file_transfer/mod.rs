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
use std::sync::Arc;

use edgehog_store::models::job::job_type::JobType;
use eyre::{Context, OptionExt};
use futures::StreamExt;
use tokio::io::AsyncSeekExt;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

use crate::controller::actor::Persisted;
use crate::file_transfer::file_system::store::Limit;
use crate::file_transfer::interface::FileTransferId;
use crate::file_transfer::interface::request::FileTransferRequest;
use crate::file_transfer::interface::status::FileTransferResponse;
use crate::http::FtHttpClient;
use crate::jobs::Queue;

use self::compression::tar_gz::TarGzWriter;
use self::file_system::FileOptions;
use self::file_system::store::{FileStorage, Fs, Space};
use self::file_system::stream::{Pipe, Streaming, SysPipe};
use self::interface::capabilities::CAPABILITIES;
use self::request::download::Download;
use self::request::upload::Upload;
use self::request::{Request, Target};

mod compression;
mod file_system;
pub(crate) mod interface;
mod request;

#[derive(Debug)]
pub(crate) struct Receiver<C> {
    queue: Queue,
    notify: Arc<Notify>,
    device: C,
}

impl<C> Receiver<C> {
    pub(crate) fn new(queue: Queue, notify: Arc<Notify>, device: C) -> Self {
        Self {
            notify,
            queue,
            device,
        }
    }
}

impl<C> Persisted for Receiver<C>
where
    C: astarte_device_sdk::Client + Send + Sync + 'static,
{
    type Msg = FileTransferRequest;
    type Event = Request;

    fn task() -> &'static str {
        "file-transfer"
    }

    fn queue(&self) -> &Queue {
        &self.queue
    }

    fn workers(&self) -> &Notify {
        &self.notify
    }

    #[instrument(skip_all)]
    async fn init(&mut self) -> eyre::Result<()> {
        CAPABILITIES.send(&mut self.device).await;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn validate_job(&mut self, msg: &Self::Msg) -> eyre::Result<Self::Event> {
        match msg {
            FileTransferRequest::Download(server_to_device) => {
                let download = Download::try_from(server_to_device)?;

                info!("file download received");

                Ok(Request::Download(download))
            }
            FileTransferRequest::Upload(device_to_server) => {
                let upload = Upload::try_from(device_to_server)?;

                info!("file upload received");

                Ok(Request::Upload(upload))
            }
        }
    }

    #[instrument(skip_all)]
    async fn fail_job(&mut self, msg: &Self::Msg, report: eyre::Report) {
        if let Err(e) = FileTransferResponse::validation_error(FileTransferId::from(msg), report)
            .send(&mut self.device)
            .await
        {
            error!(%e, "response send error")
        }
    }

    #[instrument(skip_all)]
    async fn handle_backpressure(&mut self, job: &Self::Event) {
        if let Err(e) = FileTransferResponse::busy_error(FileTransferId::from(job))
            .send(&mut self.device)
            .await
        {
            error!(%e, "response send error")
        }
    }
}

#[derive(Debug)]
pub(crate) struct FileTransfer<F, S, C> {
    queue: Queue,
    storage: FileStorage<F>,
    stream: Streaming<S>,
    client: FtHttpClient,
    device: C,
}

impl<C> FileTransfer<Fs, SysPipe, C> {
    pub fn create(queue: Queue, dir: PathBuf, device: C) -> eyre::Result<Self> {
        let client = FtHttpClient::create().wrap_err("can't construct file transfer client")?;

        Ok(Self {
            queue,
            storage: FileStorage::new(dir),
            stream: Streaming::new(),
            client,
            device,
        })
    }
}

impl<F, S, C> FileTransfer<F, S, C> {
    /// Starts the task to download the  files
    #[instrument(skip_all)]
    pub(crate) async fn run(mut self, notify: Arc<Notify>) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        self.storage.init().await?;

        // TODO cancel
        loop {
            self.jobs().await?;

            debug!("waiting for next job");

            notify.notified().await;

            info!("job notified")
        }
    }

    async fn jobs(&mut self) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        while let Some(job) = self
            .queue
            .next::<Request>(JobType::FileTransfer)
            .await
            .wrap_err("couldn't get next job")?
        {
            if let Err(error) = self.handle(job).await {
                error!(error = format!("{error:#}"), "couldn't handle job");
            }
        }

        Ok(())
    }

    #[instrument(skip_all, fields(id = %job.id()))]
    async fn handle(&mut self, job: Request) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        let id = FileTransferId::from(&job);

        match job {
            Request::Download(download) => {
                self.download(download).await?;
            }
            Request::Upload(upload) => {
                self.upload(upload).await?;
            }
        }

        self.job_done(id).await?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn download(&mut self, req: Download) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        match req.destination_type {
            Target::Storage => self.store(req).await,
            Target::Stream => self.stream(req).await,
        }
    }

    #[instrument(skip_all)]
    async fn upload(&mut self, req: Upload) -> eyre::Result<()>
    where
        S: Pipe,
    {
        match req.source_type {
            Target::Storage => {
                let id = Uuid::parse_str(&req.source)?;

                if let Some(encoding) = req.encoding {
                    let (rx, tx) = tokio::io::simplex(8 * 1024);

                    let (mut walk, base_path) = self
                        .storage
                        .walk(&id)
                        .await
                        .ok_or_eyre("file doesn't exists")?;

                    match encoding {
                        request::Encoding::TarGz => {
                            tokio::task::spawn(async move {
                                let mut writer = TarGzWriter::new(tx);

                                while let Some(item) = walk.next().await {
                                    let item = item?;

                                    writer.append(&base_path, item.path()).await?;
                                }

                                writer.finalize().await?;

                                Ok::<_, eyre::Report>(())
                            });

                            let reader = tokio_util::io::ReaderStream::new(rx);

                            let _ = self.client.upload(&req.url, req.headers, reader).await?;
                        }
                    }
                } else {
                    let file = self.storage.open_read(&id).await?;

                    let body_stream = ReaderStream::new(file);

                    let _ = self
                        .client
                        .upload(&req.url, req.headers, body_stream)
                        .await?;
                }

                Ok(())
            }
            Target::Stream => {
                let id = Uuid::parse_str(&req.source)?;

                let file = self.stream.create_reader(&id).await?;

                let body_stream = ReaderStream::new(file);

                let _ = self
                    .client
                    .upload(&req.url, req.headers, body_stream)
                    .await?;

                Ok(())
            }
        }
    }

    async fn store(&mut self, download: Download) -> eyre::Result<()>
    where
        F: Space,
    {
        if self.storage.file_exists(&download.id).await? {
            info!("file already exists");

            return Ok(());
        }

        let opt = FileOptions::from(&download);

        let (mut file, mut digest) = self.storage.create_write_handle(&opt).await?;

        let total_len = download.encoding.is_none().then_some(download.file_size);

        let mut file_resp = self
            .client
            .download(
                &download.url,
                download.headers,
                file.current_size(),
                total_len,
            )
            .await
            .wrap_err("download request error")?;

        if file_resp.start() != file.current_size() {
            file.seek(SeekFrom::Start(file_resp.start())).await?;
        }

        // limit to maximum uncompressed file size
        let limit = file_resp.total_length().unwrap_or(download.file_size);

        let mut limit = Limit::new(limit, &mut file);
        file_resp.write_chunks(&mut limit, &mut digest).await?;

        self.storage.finalize_write(file, &opt).await?;

        Ok(())
    }

    async fn stream(&mut self, download: Download) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        let opt = FileOptions::from(&download);

        let (mut file, mut digest) = self.stream.open_writer(&opt).await?;

        let total_len = download.encoding.is_none().then_some(download.file_size);

        let mut file_resp = self
            .client
            .download(&download.url, download.headers, 0, total_len)
            .await
            .wrap_err("download request error")?;

        // limit to maximum uncompressed file size
        let limit = file_resp.rem_len().unwrap_or(download.file_size);

        let mut limit = Limit::new(limit, &mut file);
        file_resp.write_chunks(&mut limit, &mut digest).await?;

        // TODO: finalize the digest

        Ok(())
    }

    async fn job_done(&mut self, id: FileTransferId) -> Result<(), eyre::Error>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        self.queue
            .update(
                id.uuid(),
                edgehog_store::models::job::status::JobStatus::Done,
            )
            .await
            .wrap_err("couldn't update job status")?;

        FileTransferResponse::success(id)
            .send(&mut self.device)
            .await
            .wrap_err("couldn't send response to astarte")?;

        self.queue
            .delete(id.uuid())
            .await
            .wrap_err("couldn't clean up job")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::iter;
    use std::time::Duration;

    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use aws_lc_rs::digest;
    use edgehog_store::db::Handle;
    use httpmock::Method::PUT;
    use httpmock::Mock;
    use httpmock::{Method::GET, MockServer};
    use minicbor::bytes::ByteVec;
    use mockall::Sequence;
    use mockall::predicate::{always, eq};
    use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
    use rstest::rstest;
    use tempdir::TempDir;
    use uuid::Uuid;

    use super::request::{FileDigest, FilePermissions};
    use super::*;

    async fn mk_transfer(
        prefix: &str,
        client: MockDeviceClient<Mqtt<SqliteStore>>,
    ) -> (
        FileTransfer<Fs, SysPipe, MockDeviceClient<Mqtt<SqliteStore>>>,
        TempDir,
    ) {
        let dir = TempDir::new(prefix).unwrap();

        let db = Handle::open(dir.path().join("state.db")).await.unwrap();

        let queue = Queue::new(db);

        (
            FileTransfer::create(queue, dir.path().to_path_buf(), client).unwrap(),
            dir,
        )
    }

    fn mk_download_req(url: &str, headers: HeaderMap, content: &[u8]) -> Request {
        let mut digest = digest::Context::new(&digest::SHA256);
        digest.update(content);
        let digest = digest.finish();

        Request::Download(Download {
            id: Uuid::new_v4(),
            url: url.parse().unwrap(),
            headers,
            progress: true,
            digest_type: FileDigest::Sha256,
            digest: ByteVec::from(digest.as_ref().to_vec()),
            ttl: Some(Duration::from_secs(60)),
            encoding: None,
            file_size: content.len().try_into().unwrap(),
            permission: FilePermissions::default(),
            destination_type: Target::Storage,
            destination: String::new(),
        })
    }

    fn mk_headers() -> HeaderMap {
        HeaderMap::from_iter([(
            AUTHORIZATION,
            HeaderValue::from_static(
                "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk811kA==",
            ),
        )])
    }

    struct MockedFtCall<'a> {
        first_call: Mock<'a>,
        second_call: Option<Mock<'a>>,
        req: Request,
        content: Vec<u8>,
    }

    impl<'a> MockedFtCall<'a> {
        fn new(call_get: Mock<'a>, event: Request, content: Vec<u8>) -> Self {
            Self {
                first_call: call_get,
                req: event,
                content,
                second_call: None,
            }
        }

        fn with_second_call(
            call_get: Mock<'a>,
            call_full_get: Mock<'a>,
            event: Request,
            content: Vec<u8>,
        ) -> Self {
            Self {
                first_call: call_get,
                req: event,
                content,
                second_call: Some(call_full_get),
            }
        }
    }

    async fn mk_download<'a>(server: &'a MockServer) -> MockedFtCall<'a> {
        let url = server.url("/file");

        let size = 1024usize;
        let content = vec![1u8; size];

        let headers = mk_headers();

        let call_get = server
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file")
                    // full
                    .header_missing("range");

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Length", "1024")
                    .body(&content);
            })
            .await;

        let req = mk_download_req(&url, headers, &content);

        MockedFtCall::new(call_get, req, content)
    }

    async fn mk_download_part<'a>(server: &'a MockServer) -> MockedFtCall<'a> {
        let url = server.url("/file-part");
        let size = 1024usize;

        let content = vec![3u8; size];
        let partial_content = content[..512].to_vec();

        let headers = mk_headers();

        let call_get = server
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part")
                    .header_includes("range", format!("bytes={}-", partial_content.len()));

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 512-1023/1024")
                    .header("Content-Length", "512")
                    .body(&partial_content);
            })
            .await;

        let req = mk_download_req(&url, headers, &content);

        MockedFtCall::new(call_get, req, content)
    }

    async fn mk_download_not_satisfied<'a>(server: &'a MockServer) -> MockedFtCall<'a> {
        let url = server.url("/file-part-not-206");
        let size = 1024usize;
        let content = vec![4u8; size];

        let headers = mk_headers();

        let call_get = server
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part-not-206")
                    .header_includes("range", "bytes=512-");

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
            })
            .await;
        let call_get_full = server
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file-part-not-206")
                    .header_missing("range");

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::OK)
                    .header("Content-Length", content.len().to_string())
                    .body(&content);
            })
            .await;

        let req = mk_download_req(&url, headers, &content);

        MockedFtCall::with_second_call(call_get, call_get_full, req, content)
    }

    fn mk_upload_req(url: &str, headers: HeaderMap) -> Request {
        Request::Upload(Upload {
            id: Uuid::new_v4(),
            url: url.parse().unwrap(),
            headers,
            progress: true,
            encoding: None,
            source_type: Target::Storage,
            source: Uuid::new_v4().to_string(),
        })
    }

    async fn mk_upload<'a>(mock_fs: &'a MockServer) -> MockedFtCall<'a> {
        let url = mock_fs.url("/file_upload");
        let size = 1024usize;
        let content: String = iter::repeat_n("ciao", size / 4).collect();

        let headers = mk_headers();

        let call_put = mock_fs
            .mock_async(|mut when, then| {
                when = when.method(PUT).path("/file_upload").body(&content);

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::OK);
            })
            .await;

        let req = mk_upload_req(&url, headers);

        MockedFtCall::new(call_put, req, content.into_bytes())
    }

    #[rstest]
    #[tokio::test]
    async fn should_download() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download(&server).await;

        let mut seq = Sequence::new();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        device
            .expect_send_object()
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Response"),
                eq("/request"),
                always(),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        let (mut transfer, dir) = mk_transfer("should_download", device).await;

        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.first_call.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_part() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download_part(&server).await;

        let mut seq = Sequence::new();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        device
            .expect_send_object()
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Response"),
                eq("/request"),
                always(),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        let (mut transfer, dir) = mk_transfer("download", device).await;

        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.req.id()));
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.first_call.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_full_when_unsatisfiable() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download_not_satisfied(&server).await;

        let mut seq = Sequence::new();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        device
            .expect_send_object()
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Response"),
                eq("/request"),
                always(),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        let (mut transfer, dir) = mk_transfer("download_full_unsatisfiable", device).await;
        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.req.id()));
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.first_call.assert_async().await;
        mock_download_event
            .second_call
            .unwrap()
            .assert_async()
            .await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_upload() {
        let server = MockServer::start_async().await;
        let mock_upload_event = mk_upload(&server).await;

        let mut seq = Sequence::new();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        device
            .expect_send_object()
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Response"),
                eq("/request"),
                always(),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        let (mut transfer, dir) = mk_transfer("upload", device).await;

        let Request::Upload(Upload { source, .. }) = &mock_upload_event.req else {
            unreachable!()
        };
        tokio::fs::write(dir.path().join(source), mock_upload_event.content)
            .await
            .unwrap();

        transfer.handle(mock_upload_event.req).await.unwrap();

        mock_upload_event.first_call.assert_async().await;
    }

    #[rstest]
    #[tokio::test]
    async fn update_job() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download(&server).await;

        let mut seq = Sequence::new();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        device
            .expect_send_object()
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Response"),
                eq("/request"),
                always(),
            )
            .once()
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(()));
        let (mut transfer, dir) = mk_transfer("update_job", device).await;
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        transfer
            .queue
            .insert(&mock_download_event.req)
            .await
            .unwrap();

        transfer.jobs().await.unwrap();

        mock_download_event.first_call.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);

        let job = transfer.queue.fetch_job(mock_download_event.req.id()).await;

        assert!(job.is_none());
    }
}
