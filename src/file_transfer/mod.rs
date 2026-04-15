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

//! Transfer files from and to the Device

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use astarte_device_sdk::Client;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::Context;
use tokio::sync::{Notify, watch};
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, trace, warn};
use uuid::Uuid;

use crate::controller::actor::Persisted;
use crate::file_transfer::http::FtHttpClient;
use crate::file_transfer::interface::request::FileTransferRequest;
use crate::file_transfer::interface::status::FileTransferProgress;
use crate::io::limit::Limit;
use crate::io::progress::{Progress, ProgressHandle};
use crate::jobs::Queue;
use crate::{file_transfer::interface::status::FileTransferResponse, io::digest::Digest};

use self::encoding::Paths;
use self::encoding::tar_gz::TarGzBuilder;
use self::file_system::store::{FileStorage, Fs, Space};
use self::file_system::stream::{Pipe, Streaming, SysPipe};
use self::file_system::{FileOptions, WriteHandle};
use self::interface::capabilities::CAPABILITIES;
use self::interface::status::FileTransferId;
use self::request::download::{Destination, Download};
use self::request::upload::{Source, Upload};
use self::request::{Encoding, JobTag, Request};

mod encoding;
pub(crate) mod file_system;
pub(crate) mod http;
pub(crate) mod interface;
pub(crate) mod request;

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
    C: Client + Send + Sync + 'static,
{
    type Msg = FileTransferRequest;

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
    async fn validate_job(&mut self, msg: &Self::Msg) -> eyre::Result<Job> {
        match msg {
            FileTransferRequest::Download(server_to_device) => {
                let download = Download::try_from(server_to_device)?;

                info!("file download received");

                Job::try_from(download)
            }
            FileTransferRequest::Upload(device_to_server) => {
                let upload = Upload::try_from(device_to_server)?;

                info!("file upload received");

                Job::try_from(upload)
            }
        }
    }

    #[instrument(skip_all)]
    async fn fail_job(&mut self, msg: &Self::Msg, report: eyre::Report) {
        let (id, direction) = msg.resp_id();

        let send = FileTransferResponse::validation_error(id, direction, report)
            .send(&mut self.device)
            .await;

        if let Err(error) = send {
            error!(%error, "response send error")
        }
    }

    #[instrument(skip_all)]
    async fn handle_backpressure(&mut self, message: &Self::Msg) {
        let (id, direction) = message.resp_id();

        let send = FileTransferResponse::busy_error(id, direction)
            .send(&mut self.device)
            .await;

        if let Err(error) = send {
            error!(%error, "response send error")
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
    tracker: watch::Sender<Option<FileTransferProgress>>,
}

impl<C> FileTransfer<Fs, SysPipe, C> {
    pub fn create(
        queue: Queue,
        dir: PathBuf,
        device: C,
        tracker: watch::Sender<Option<FileTransferProgress>>,
    ) -> eyre::Result<Self>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        let client = FtHttpClient::create().wrap_err("can't construct file transfer client")?;

        Ok(Self {
            queue,
            storage: FileStorage::new(dir),
            stream: Streaming::new(),
            client,
            device,
            tracker,
        })
    }
}

impl<F, S, C> FileTransfer<F, S, C> {
    /// Starts the task to download the  files
    #[instrument(skip_all)]
    pub(crate) async fn run(
        mut self,
        notify: Arc<Notify>,
        cancel: CancellationToken,
    ) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: Client + Send + Sync + 'static,
    {
        self.storage.init().await?;

        self.jobs(&cancel).await?;

        while Self::notified(&notify, &cancel).await {
            info!("job notified");

            self.jobs(&cancel).await?;

            debug!("waiting for next job");
        }

        Ok(())
    }

    async fn notified(notify: &Notify, cancel: &CancellationToken) -> bool {
        cancel
            .run_until_cancelled(notify.notified())
            .await
            .is_some()
    }

    async fn next_job(&mut self) -> eyre::Result<Option<Request<'static>>> {
        self.queue
            .next::<Request>(JobType::FileTransfer)
            .await
            .wrap_err("couldn't get next job")
    }

    async fn jobs(&mut self, cancel: &CancellationToken) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        while let Some(job) = self.next_job().await?
            && !cancel.is_cancelled()
        {
            if let Err(error) = self.handle(job).await {
                error!(error = format!("{error:#}"), "couldn't handle job");
            }
        }

        Ok(())
    }

    #[instrument(skip_all, fields(id = %job.id()))]
    async fn handle(&mut self, job: Request<'_>) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
        C: Client + Send + Sync + 'static,
    {
        let transfer = match job {
            Request::Download(download) => {
                self.download(&download).await?;

                download.transfer()
            }
            Request::Upload(upload) => {
                self.upload(&upload).await?;

                upload.transfer()
            }
        };

        self.job_done(transfer).await?;

        Ok(())
    }

    #[instrument(skip_all)]
    async fn download(&mut self, req: &Download<'_>) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        match &req.destination {
            Destination::Storage => self.download_store(req).await,
            Destination::Stream => self.download_stream(req).await,
            Destination::FileSystem { path } => {
                self.download_filesystem(path.clone().into(), req).await
            }
        }
    }

    async fn download_store(&mut self, download: &Download<'_>) -> eyre::Result<()>
    where
        F: Space,
    {
        let exists = self
            .storage
            .file_exists(&download.id, download.digest_type, &download.digest)
            .await?;

        if exists {
            info!("file already exists");

            return Ok(());
        }

        let opt = FileOptions::from(download);

        let file = self.storage.create_write_handle(&opt).await?;

        let file = self.download_to_write_handle(download, file).await?;

        self.storage.finalize_write(file, &opt).await?;

        Ok(())
    }

    async fn download_stream(&mut self, download: &Download<'_>) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        let opt = FileOptions::from(download);

        let pipe = self.stream.open_writer(&opt).await?;

        let download_len = download.download_length();

        let mut file_resp = self
            .client
            .download(&download.url, download.headers.clone(), 0, download_len)
            .await
            .wrap_err("download request error")?;

        let limit_size = file_resp.total_length().unwrap_or(download.file_size);
        let progress = FileTransferProgress::start(
            download.transfer(),
            download.progress,
            file_resp.total_length(),
        );

        // limit to maximum uncompressed file size
        let writer = Digest::new(pipe, download.digest_type);
        let writer = Limit::new(writer, limit_size);
        let mut writer = ProgressUpdate::track(writer, progress, &self.tracker);

        file_resp.write_chunks(&mut writer).await?;

        writer
            .into_inner()
            .into_inner()
            .into_inner(&download.digest)?;

        Ok(())
    }

    async fn download_filesystem(
        &mut self,
        path: PathBuf,
        download: &Download<'_>,
    ) -> eyre::Result<()>
    where
        F: Space,
    {
        if WriteHandle::try_exists(&path, download.digest_type, &download.digest).await? {
            info!("file already exists");

            return Ok(());
        }

        let opt = FileOptions::from(download);

        let file = WriteHandle::open(path, &opt).await?;

        let file = self.download_to_write_handle(download, file).await?;

        file.finalize(&opt).await?;

        Ok(())
    }

    async fn download_to_write_handle(
        &mut self,
        download: &Download<'_>,
        file: WriteHandle,
    ) -> eyre::Result<WriteHandle> {
        let current_size = file.current_size();
        let download_length = download.download_length();

        let mut file_resp = self
            .client
            .download(
                &download.url,
                download.headers.clone(),
                current_size,
                download_length,
            )
            .await
            .wrap_err("download request error")?;

        let limit_size = file_resp.total_length().unwrap_or(download.file_size);
        let progress = FileTransferProgress::start(
            download.transfer(),
            download.progress,
            file_resp.total_length(),
        );

        let writer = Digest::from_read(file, download.digest_type, file_resp.start()).await?;
        let writer = Limit::new(writer, limit_size);
        let mut writer = ProgressUpdate::track(writer, progress, &self.tracker);

        file_resp.write_chunks(&mut writer).await?;

        let file = writer
            .into_inner()
            .into_inner()
            .into_inner(&download.digest)?;

        Ok(file)
    }

    #[instrument(skip_all)]
    async fn upload(&mut self, req: &Upload<'_>) -> eyre::Result<()>
    where
        S: Pipe,
    {
        match &req.source {
            Source::Storage { id } => self.upload_store(id, req).await,
            Source::Stream => self.upload_stream(req).await,
            Source::FileSystem { path } => self.upload_filesystem(path, req).await,
        }
    }

    #[instrument(skip_all, fields(%id))]
    async fn upload_store(&mut self, id: &Uuid, req: &Upload<'_>) -> eyre::Result<()> {
        match req.encoding {
            Some(Encoding::TarGz) => {
                let paths = self.storage.find_paths(id).await?;

                let progress = FileTransferProgress::start(req.transfer(), req.progress, None);

                let reader = TarGzBuilder::spawn(paths);
                let reader = ProgressUpdate::track(reader, progress, &self.tracker);
                let reader = ReaderStream::new(reader);

                self.client
                    .upload(&req.url, req.headers.clone(), reader)
                    .await?;
            }
            None => {
                let reader = self.storage.open_read(id).await?;

                let file_len = reader.metadata().await?.len();
                let progress =
                    FileTransferProgress::start(req.transfer(), req.progress, Some(file_len));

                let reader = ProgressUpdate::track(reader, progress, &self.tracker);
                let reader = ReaderStream::new(reader);

                self.client
                    .upload(&req.url, req.headers.clone(), reader)
                    .await?;
            }
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn upload_stream(&mut self, req: &Upload<'_>) -> eyre::Result<()>
    where
        S: Pipe,
    {
        let reader = self.stream.create_reader(&req.id).await?;

        let progress = FileTransferProgress::start(req.transfer(), req.progress, None);

        let reader = ProgressUpdate::track(reader, progress, &self.tracker);
        let reader = ReaderStream::new(reader);

        self.client
            .upload(&req.url, req.headers.clone(), reader)
            .await?;

        Ok(())
    }

    #[instrument(skip_all, fields(path = %path.display()))]
    async fn upload_filesystem(&mut self, path: &Path, req: &Upload<'_>) -> eyre::Result<()> {
        match req.encoding {
            Some(Encoding::TarGz) => {
                let paths = Paths::read(path.to_path_buf()).await?;

                let progress = FileTransferProgress::start(req.transfer(), req.progress, None);

                let reader = TarGzBuilder::spawn(paths);
                let reader = ProgressUpdate::track(reader, progress, &self.tracker);
                let reader = tokio_util::io::ReaderStream::new(reader);

                self.client
                    .upload(&req.url, req.headers.clone(), reader)
                    .await?;
            }
            None => {
                let reader = tokio::fs::File::open(path).await?;

                let file_len = reader.metadata().await?.len();
                let progress =
                    FileTransferProgress::start(req.transfer(), req.progress, Some(file_len));

                let reader = ProgressUpdate::track(reader, progress, &self.tracker);
                let reader = ReaderStream::new(reader);

                self.client
                    .upload(&req.url, req.headers.clone(), reader)
                    .await?;
            }
        }

        Ok(())
    }

    async fn job_done(&mut self, transfer: FileTransferId) -> Result<(), eyre::Error>
    where
        C: Client + Send + Sync + 'static,
    {
        let FileTransferId { id, direction } = transfer;

        let tag = JobTag::from(direction);
        self.queue
            .update(&id, tag.into(), JobStatus::Done)
            .await
            .wrap_err("couldn't update job status")?;

        FileTransferResponse::success(transfer)
            .send(&mut self.device)
            .await
            .wrap_err("couldn't send response to astarte")?;

        self.queue
            .delete(&id, tag.into())
            .await
            .wrap_err("couldn't clean up job")?;

        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ProgressTracker<C> {
    device: C,
}

impl<C> ProgressTracker<C> {
    pub(crate) fn create(device: C) -> Self {
        Self { device }
    }

    pub(crate) async fn run(
        mut self,
        mut rx: watch::Receiver<Option<FileTransferProgress>>,
        cancel: CancellationToken,
    ) -> eyre::Result<()>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        while Self::changed(&mut rx, &cancel).await {
            // NOTE clone to avoid keeping the reference during an await point
            let event = rx.borrow_and_update().clone();

            if let Some(p) = event {
                trace!(?p, "transfer progress event");

                if let Err(e) = p.send(&mut self.device).await {
                    error!(err=%e, "error while sending progress to astarte");
                }
            }
        }

        Ok(())
    }

    async fn changed(
        rx: &mut watch::Receiver<Option<FileTransferProgress>>,
        cancel: &CancellationToken,
    ) -> bool {
        let res = cancel.run_until_cancelled(rx.changed()).await;

        matches!(res, Some(Ok(())))
    }
}

type Watch = watch::Sender<Option<FileTransferProgress>>;

#[derive(Debug)]
struct ProgressUpdate {
    tx: Option<Watch>,
}

impl ProgressUpdate {
    fn track<T>(
        inner: T,
        progress: Option<FileTransferProgress>,
        sender: &Watch,
    ) -> Progress<T, Self> {
        let this = if let Some(start) = progress {
            sender.send_modify(|p| *p = Some(start));

            Self {
                tx: Some(sender.clone()),
            }
        } else {
            Self { tx: None }
        };

        Progress::new(inner, this)
    }
}

impl ProgressHandle for ProgressUpdate {
    fn update(&mut self, bytes: u64) -> io::Result<()> {
        let Some(sender) = &self.tx else {
            return Ok(());
        };

        let modified = sender.send_if_modified(|old| {
            if let Some(old) = old {
                old.set_bytes(bytes);

                true
            } else {
                warn!("file transfer removed from channel");

                false
            }
        });

        if modified {
            Ok(())
        } else {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use astarte_device_sdk::aggregate::AstarteObject;
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
    use mockall::predicate::{always, eq, function};
    use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
    use rstest::rstest;
    use tempdir::TempDir;
    use uuid::Uuid;

    use super::request::{FileDigest, FilePermissions};
    use super::*;

    async fn mk_def_transfer(
        prefix: &str,
        client: MockDeviceClient<Mqtt<SqliteStore>>,
    ) -> (
        FileTransfer<Fs, SysPipe, MockDeviceClient<Mqtt<SqliteStore>>>,
        TempDir,
    ) {
        let (tx, _rx) = watch::channel(None);

        mk_transfer(prefix, client, tx).await
    }

    async fn mk_transfer(
        prefix: &str,
        device: MockDeviceClient<Mqtt<SqliteStore>>,
        tracker: watch::Sender<Option<FileTransferProgress>>,
    ) -> (
        FileTransfer<Fs, SysPipe, MockDeviceClient<Mqtt<SqliteStore>>>,
        TempDir,
    ) {
        let dir = TempDir::new(prefix).unwrap();

        let db = Handle::open(dir.path().join("state.db")).await.unwrap();

        let queue = Queue::new(db);

        (
            FileTransfer::create(queue, dir.path().to_path_buf(), device, tracker).unwrap(),
            dir,
        )
    }

    fn mk_download_req(url: &str, headers: HeaderMap, content: &[u8]) -> Request<'static> {
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
            destination: Destination::Storage,
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
        req: Request<'a>,
        content: Vec<u8>,
    }

    impl<'a> MockedFtCall<'a> {
        fn new(call_get: Mock<'a>, event: Request<'a>, content: Vec<u8>) -> Self {
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
            event: Request<'a>,
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

    fn mk_upload_req(url: &str, headers: HeaderMap) -> Request<'static> {
        Request::Upload(Upload {
            id: Uuid::new_v4(),
            url: url.parse().unwrap(),
            headers,
            progress: true,
            encoding: None,
            source: Source::Storage { id: Uuid::new_v4() },
        })
    }

    async fn mk_upload<'a>(mock_fs: &'a MockServer) -> MockedFtCall<'a> {
        let url = mock_fs.url("/file_upload");
        let size = 1024usize;
        let content = "a".repeat(size);

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

    async fn mk_large_upload<'a>(mock_fs: &'a MockServer) -> MockedFtCall<'a> {
        let url = mock_fs.url("/file_upload");
        let size = 1 << 15;
        let content: String = std::iter::repeat_n("ciao", size / 4).collect();

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
        let (mut transfer, dir) = mk_def_transfer("should_download", device).await;

        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.first_call.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

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
        let (mut transfer, dir) = mk_def_transfer("download", device).await;

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
        let (mut transfer, dir) = mk_def_transfer("download_full_unsatisfiable", device).await;
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
        let (mut transfer, dir) = mk_def_transfer("upload", device).await;

        tokio::fs::write(
            dir.path().join(mock_upload_event.req.target()),
            mock_upload_event.content,
        )
        .await
        .unwrap();

        transfer.handle(mock_upload_event.req).await.unwrap();

        mock_upload_event.first_call.assert_async().await;
    }

    #[tokio::test]
    async fn update_job_download() {
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
        let (mut transfer, dir) = mk_def_transfer("update_job", device).await;
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        let Request::Download(download) = mock_download_event.req.clone() else {
            unreachable!()
        };
        transfer.queue.insert(download).await.unwrap();

        transfer.jobs(&CancellationToken::new()).await.unwrap();

        mock_download_event.first_call.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);

        let job = transfer.queue.fetch_job(mock_download_event.req.id()).await;

        assert!(job.is_none());
    }

    #[tokio::test]
    async fn should_report_progress() {
        let server = MockServer::start_async().await;
        let mock_upload_event = mk_large_upload(&server).await;

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
        let mut progress_device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        progress_device
            .expect_send_object()
            .times(1..)
            .with(
                eq("io.edgehog.devicemanager.fileTransfer.Progress"),
                eq("/request"),
                function(|e: &AstarteObject| {
                    let Some(total) = e.get("totalBytes").and_then(|d| match d {
                        &astarte_device_sdk::AstarteData::LongInteger(l) => Some(l),
                        _ => None,
                    }) else {
                        return false;
                    };

                    let total = match total {
                        -1 => None,
                        n @ 0.. => Some(u64::try_from(n).unwrap()),
                        n => panic!("unexpected value '{}'", n),
                    };

                    let Some(bytes) = e.get("bytes").and_then(|d| match d {
                        &astarte_device_sdk::AstarteData::LongInteger(l) => Some(l),
                        _ => None,
                    }) else {
                        return false;
                    };

                    let bytes = u64::try_from(bytes).unwrap();

                    info!("reported progress {bytes}/{total:?}");

                    total.is_none_or(|t| bytes <= t)
                }),
            )
            .returning(|_, _, _| Ok(()));
        let (tx_progress, rx_progress) = watch::channel(None);
        let (mut transfer, dir) = mk_transfer("upload", device, tx_progress).await;

        let Request::Upload(Upload {
            source: Source::Storage { id },
            ..
        }) = &mock_upload_event.req
        else {
            unreachable!()
        };

        tokio::fs::write(dir.path().join(id.to_string()), mock_upload_event.content)
            .await
            .unwrap();

        let cancel = CancellationToken::new();

        let handle = tokio::spawn(
            ProgressTracker::create(progress_device).run(rx_progress, cancel.child_token()),
        );

        transfer.handle(mock_upload_event.req).await.unwrap();

        cancel.cancel();
        handle.await.unwrap().unwrap();

        mock_upload_event.first_call.assert_async().await;
    }

    #[rstest]
    #[tokio::test]
    async fn update_job_upload() {
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
        let (tx_progress, _rx_progress) = watch::channel(None);
        let (mut transfer, dir) = mk_transfer("upload", device, tx_progress).await;

        tokio::fs::write(
            dir.path().join(mock_upload_event.req.target()),
            mock_upload_event.content,
        )
        .await
        .unwrap();

        let Request::Upload(download) = mock_upload_event.req.clone() else {
            unreachable!()
        };
        transfer.queue.insert(download).await.unwrap();

        transfer.jobs(&CancellationToken::new()).await.unwrap();

        mock_upload_event.first_call.assert_async().await;

        let job = transfer.queue.fetch_job(mock_upload_event.req.id()).await;

        assert!(job.is_none());
    }
}
