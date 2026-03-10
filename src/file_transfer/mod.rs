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

use aws_lc_rs::digest;
use edgehog_store::models::job::job_type::JobType;
use eyre::{Context, OptionExt};
use tokio::io::{AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Notify;
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

use crate::controller::actor::Persisted;
use crate::http::{DownloadRange, FileDownloadResponse, FileTransferHttpClient};
use crate::jobs::Queue;

use self::file_system::FileOptions;
use self::file_system::store::{FileStorage, Fs, Space};
use self::file_system::stream::{Pipe, Streaming, SysPipe};
use self::interface::FileTransferEvent;
use self::request::download::Download;
use self::request::upload::Upload;
use self::request::{Request, Target};

mod file_system;
pub(crate) mod interface;
mod request;

#[derive(Debug)]
pub(crate) struct Receiver {
    queue: Queue,
    notify: Arc<Notify>,
}

impl Receiver {
    pub(crate) fn new(queue: Queue, notify: Arc<Notify>) -> Self {
        Self { notify, queue }
    }
}

impl Persisted for Receiver {
    type Msg = FileTransferEvent;
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
        Ok(())
    }

    #[instrument(skip_all)]
    async fn validate_job(&mut self, msg: &Self::Msg) -> eyre::Result<Self::Event> {
        match msg {
            FileTransferEvent::Download(server_to_device) => {
                let download = Download::try_from(server_to_device)?;

                info!("file download received");

                Ok(Request::Download(download))
            }
            FileTransferEvent::Upload(device_to_server) => {
                let upload = Upload::try_from(device_to_server)?;

                info!("file upload received");

                Ok(Request::Upload(upload))
            }
        }
    }

    #[instrument(skip_all)]
    async fn fail_job(&mut self, _msg: &Self::Msg) {
        unimplemented!("send error to Astarte")
    }

    #[instrument(skip_all)]
    async fn handle_backpressure(&mut self, _job: &Self::Event) {
        unimplemented!("send error to Astarte")
    }
}

#[derive(Debug)]
pub(crate) struct FileTransfer<F, S> {
    queue: Queue,
    storage: FileStorage<F>,
    stream: Streaming<S>,
    client: FileTransferHttpClient,
}

impl FileTransfer<Fs, SysPipe> {
    pub fn create(queue: Queue, dir: PathBuf) -> eyre::Result<Self> {
        let client =
            FileTransferHttpClient::create().wrap_err("can't construct file transfer client")?;

        Ok(Self {
            queue,
            storage: FileStorage::new(dir),
            stream: Streaming::new(),
            client,
        })
    }
}

impl<F, S> FileTransfer<F, S> {
    /// Starts the task to download the  files
    #[instrument(skip_all)]
    pub(crate) async fn run(mut self, notify: Arc<Notify>) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
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
    {
        while let Some(job) = self
            .queue
            .next::<Request>(JobType::FileTransfer)
            .await
            .wrap_err("couldn't get next job")?
        {
            if let Err(error) = self.handle(job).await {
                error!(%error, "couldn't handle job");
            }
        }

        Ok(())
    }

    #[instrument(skip_all, fields(id = %job.id()))]
    async fn handle(&mut self, job: Request) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        match job {
            Request::Download(download) => self.download(download).await,
            Request::Upload(upload) => self.upload(upload).await,
        }
    }

    #[instrument(skip_all)]
    async fn download(&mut self, req: Download) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        match req.destination {
            Target::Storage => self.store(req).await,
            Target::Stream => self.stream(req).await,
        }
    }

    #[instrument(skip_all)]
    async fn upload(&mut self, req: Upload) -> eyre::Result<()>
    where
        S: Pipe,
    {
        match req.source {
            Target::Storage => {
                let _file = self.storage.open_read(&req.id).await?;

                unimplemented!("file upload")
            }
            Target::Stream => {
                let _pipe = self.stream.create_reader(&req.id).await?;

                unimplemented!("file upload")
            }
        }
    }

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

        let download_range = DownloadRange::create(file.current_size(), opt.file_size)
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

        self.storage.finalize_write(file, &opt).await?;

        self.job_done(&download.id).await?;

        Ok(())
    }

    async fn stream(&mut self, download: Download) -> eyre::Result<()>
    where
        F: Space,
        S: Pipe,
    {
        let opt = FileOptions::from(&download);

        let (mut file, mut digest) = self.stream.open_writer(&opt).await?;

        let range = DownloadRange::with_size(opt.file_size);

        let file_resp = self
            .client
            .download(&download.url, download.headers, range)
            .await
            .wrap_err("download request error")?;

        Self::write_chunks(&mut file, &mut digest, file_resp).await?;

        // TODO: finalize the digest

        self.job_done(&download.id).await?;

        Ok(())
    }

    async fn job_done(&mut self, id: &Uuid) -> Result<(), eyre::Error> {
        self.queue
            .update(id, edgehog_store::models::job::status::JobStatus::Done)
            .await
            .wrap_err("couldn't update job status")?;

        // TODO send to astarte

        self.queue
            .delete(id)
            .await
            .wrap_err("couldn't clean up job")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use edgehog_store::db::Handle;
    use httpmock::Mock;
    use httpmock::{Method::GET, MockServer};
    use minicbor::bytes::ByteVec;
    use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
    use rstest::rstest;
    use tempdir::TempDir;
    use uuid::Uuid;

    use crate::file_transfer::request::upload::tests::upload_req;

    use super::request::{FileDigest, FilePermissions};
    use super::*;

    async fn mk_transfer(prefix: &str) -> (FileTransfer<Fs, SysPipe>, TempDir) {
        let dir = TempDir::new(prefix).unwrap();

        let db = Handle::open(dir.path().join("state.db")).await.unwrap();

        let queue = Queue::new(db);

        (
            FileTransfer::create(queue, dir.path().to_path_buf()).unwrap(),
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
            compression: None,
            file_size: content.len().try_into().unwrap(),
            permission: FilePermissions::default(),
            destination: Target::Storage,
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

    struct MockedFsCall<'a> {
        call_get: Mock<'a>,
        call_full_get: Option<Mock<'a>>,
        req: Request,
        content: Vec<u8>,
    }

    impl<'a> MockedFsCall<'a> {
        fn new(call_get: Mock<'a>, event: Request, content: Vec<u8>) -> Self {
            Self {
                call_get,
                req: event,
                content,
                call_full_get: None,
            }
        }

        fn with_get_full(
            call_get: Mock<'a>,
            call_full_get: Mock<'a>,
            event: Request,
            content: Vec<u8>,
        ) -> Self {
            Self {
                call_get,
                req: event,
                content,
                call_full_get: Some(call_full_get),
            }
        }
    }

    async fn mk_download<'a>(server: &'a MockServer) -> MockedFsCall<'a> {
        let url = server.url("/file");

        let size = 1024usize;
        let content = vec![1u8; size];

        let headers = mk_headers();

        let call_get = server
            .mock_async(|mut when, then| {
                when = when
                    .method(GET)
                    .path("/file")
                    .header_includes("range", "bytes=0-");

                headers.iter().fold(when, |when, (n, s)| {
                    when.header_includes(n.as_str(), s.to_str().unwrap())
                });

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 0-1023/1024")
                    .body(&content);
            })
            .await;

        let req = mk_download_req(&url, headers, &content);

        MockedFsCall::new(call_get, req, content)
    }

    async fn mk_download_part<'a>(server: &'a MockServer) -> MockedFsCall<'a> {
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
                    .body(&partial_content);
            })
            .await;

        let req = mk_download_req(&url, headers, &content);

        MockedFsCall::new(call_get, req, content)
    }

    async fn mock_fs_download_part_not_satisfied<'a>(server: &'a MockServer) -> MockedFsCall<'a> {
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

        MockedFsCall::with_get_full(call_get, call_get_full, req, content)
    }

    #[rstest]
    #[tokio::test]
    async fn should_download() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download(&server).await;

        let (mut transfer, dir) = mk_transfer("should_download").await;
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.call_get.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_part() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download_part(&server).await;

        let (mut transfer, dir) = mk_transfer("download").await;
        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.req.id()));
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer.handle(mock_download_event.req).await.unwrap();

        mock_download_event.call_get.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);
    }

    #[rstest]
    #[tokio::test]
    async fn should_download_full_when_unsatisfiable() {
        let server = MockServer::start_async().await;
        let mock_download_event = mock_fs_download_part_not_satisfied(&server).await;

        let (mut transfer, dir) = mk_transfer("download_full_unsatisfiable").await;
        let partial_file = dir
            .path()
            .join(format!("{}.part", mock_download_event.req.id()));
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        // write partial file
        tokio::fs::write(&partial_file, &mock_download_event.content[0..512])
            .await
            .unwrap();

        transfer.handle(mock_download_event.req).await.unwrap();

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
    async fn should_upload(upload_req: Upload) {
        let (mut transfer, dir) = mk_transfer("upload").await;

        tokio::fs::write(dir.path().join(upload_req.id.to_string()), "Hello world!")
            .await
            .unwrap();

        transfer.handle(Request::Upload(upload_req)).await.unwrap();
    }

    #[rstest]
    #[tokio::test]
    async fn update_job() {
        let server = MockServer::start_async().await;
        let mock_download_event = mk_download(&server).await;

        let (mut transfer, dir) = mk_transfer("update_job").await;
        let complete_file = dir.path().join(mock_download_event.req.id().to_string());

        transfer
            .queue
            .insert(&mock_download_event.req)
            .await
            .unwrap();

        transfer.jobs().await.unwrap();

        mock_download_event.call_get.assert_async().await;

        let content = tokio::fs::read(complete_file).await.unwrap();

        assert_eq!(content, mock_download_event.content);

        let job = transfer.queue.fetch_job(mock_download_event.req.id()).await;

        assert!(job.is_none());
    }
}
