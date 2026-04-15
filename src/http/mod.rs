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

//! Handles an http client to download part of or an entire file.

use std::{fmt::Debug, ops::RangeInclusive};

use bytes::Bytes;
use eyre::{Context, OptionExt, eyre};
use futures::TryStream;
use reqwest::{
    StatusCode,
    header::{GetAll, HeaderMap, HeaderValue},
    redirect::Policy,
};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, instrument, trace, warn};
use url::Url;

pub fn default_http_client_builder() -> Result<reqwest::ClientBuilder, rustls::Error> {
    // TODO move tls initialization in the main function to keep a single instance
    let tls = edgehog_tls::config()?;
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(Policy::limited(3))
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ));

    Ok(client)
}

#[derive(Debug)]
pub enum FileTransferRange {
    Content {
        range_start: u64,
        range_total: Option<u64>,
    },
    RangeNotSatisfiable {
        range_total: Option<u64>,
    },
}

/// Create the http client
#[derive(Debug)]
pub struct FtHttpClient {
    client: reqwest::Client,
}

impl FtHttpClient {
    pub fn create() -> eyre::Result<Self> {
        let client = default_http_client_builder()
            .wrap_err("failed to build TLS config")?
            .build()
            .wrap_err("failed to build HTTP client")?;

        Ok(Self { client })
    }

    // TODO: Add total if available
    pub async fn download(
        &self,
        url: &Url,
        mut headers: HeaderMap,
        start: u64,
        total_len: Option<u64>,
    ) -> eyre::Result<FileDownloadResponse> {
        if start == 0 {
            return self.full_request(url, headers, total_len).await;
        }

        headers.insert(
            reqwest::header::RANGE,
            HeaderValue::from_str(&format!("bytes={}-", start))
                .wrap_err("invalid range header value")?,
        );

        let response = self
            .client
            .get(url.as_str())
            .headers(headers.clone())
            .send()
            .await
            .wrap_err("get range request error")?;

        match Self::check_response(&response)? {
            FileTransferRange::Content {
                range_start,
                range_total,
            } => {
                if range_start < start {
                    warn!(exp = start, got = range_start, "range before current size");
                } else if range_start > start {
                    return Err(eyre!(
                        "invalid range start exp ({start}), but got ({range_start})"
                    ));
                }

                let total_length = Self::check_total(total_len, range_total)?;

                Ok(FileDownloadResponse {
                    response,
                    start: range_start,
                    total_length,
                })
            }
            FileTransferRange::RangeNotSatisfiable { range_total } => {
                let total_length = Self::check_total(total_len, range_total)?;

                self.full_request(url, headers, total_length).await
            }
        }
    }

    pub async fn upload<S>(
        &self,
        url: &Url,
        headers: HeaderMap,
        body_stream: S,
    ) -> eyre::Result<FileUploadResponse>
    where
        S: TryStream + Send + 'static,
        S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
        Bytes: From<S::Ok>,
    {
        self.client
            .put(url.as_str())
            .headers(headers)
            .body(reqwest::Body::wrap_stream(body_stream))
            .send()
            .await
            .wrap_err("error while sending put upload request")?
            .error_for_status()
            .wrap_err("error in response status of put request")?;

        Ok(FileUploadResponse)
    }

    async fn full_request(
        &self,
        url: &Url,
        mut headers: HeaderMap,
        total_len: Option<u64>,
    ) -> eyre::Result<FileDownloadResponse> {
        headers.remove(reqwest::header::RANGE);

        let response = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await
            .wrap_err("get request error")?
            .error_for_status()?;

        let content_length = Self::content_length(&response)?;

        let total_length = Self::check_total(total_len, content_length)?;

        Ok(FileDownloadResponse {
            response,
            start: 0,
            total_length,
        })
    }

    fn check_total(total_len: Option<u64>, range_total: Option<u64>) -> eyre::Result<Option<u64>> {
        if let (Some(tot), Some(range)) = (total_len, range_total)
            && tot != range
        {
            return Err(eyre!("content length and total length mismatch"));
        }

        Ok(total_len.or(range_total))
    }

    fn content_length(response: &reqwest::Response) -> eyre::Result<Option<u64>> {
        let length = Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_LENGTH));

        match length {
            Ok(l) => {
                let server_len = Self::parse_content_length(l)?;

                Ok(Some(server_len))
            }
            Err(err) => {
                warn!(%err, "content length not provided by the server");

                Ok(None)
            }
        }
    }

    #[instrument(err)]
    fn check_response(response: &reqwest::Response) -> eyre::Result<FileTransferRange> {
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                trace!("received partial content status");

                let (content_range, total) =
                    Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_RANGE))
                        .and_then(Self::parse_content_range)?;

                let resp_start = content_range.start();
                // byte range end + 1
                let range_end = content_range
                    .end()
                    .checked_add(1)
                    .ok_or_eyre("range end overflow")?;

                if let Some(total) = total
                    && total != range_end
                {
                    return Err(eyre!(
                        "range end ({range_end}) is not the total length ({total})"
                    ));
                }

                Ok(FileTransferRange::Content {
                    range_start: *resp_start,
                    range_total: total,
                })
            }
            StatusCode::OK => {
                debug!("status ok, the response is not partial");

                let content_length = Self::content_length(response)?;

                Ok(FileTransferRange::Content {
                    range_start: 0,
                    range_total: content_length,
                })
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total =
                    Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_RANGE))
                        .and_then(Self::parse_unsatisfiable_content_range)
                        .inspect_err(|error| error!(%error, "couldn't get content range"))
                        .ok();

                Ok(FileTransferRange::RangeNotSatisfiable { range_total: total })
            }
            status => Err(eyre!("unexpected status code {status}")),
        }
    }

    fn expect_one(values: GetAll<'_, HeaderValue>) -> eyre::Result<&HeaderValue> {
        let mut iter = values.iter();
        let first = iter.next().ok_or_eyre("missing header")?;

        eyre::ensure!(iter.next().is_none(), "multiple headers");

        Ok(first)
    }

    #[instrument(ret)]
    fn parse_content_range(
        value: &HeaderValue,
    ) -> eyre::Result<(RangeInclusive<u64>, Option<u64>)> {
        let value = value
            .to_str()?
            .strip_prefix("bytes ")
            .ok_or_eyre("expected bytes range")?;

        let ((start, end), total) = value
            .split_once('/')
            .and_then(|(range, total)| {
                range
                    .split_once('-')
                    .map(|(start, end)| ((start, end), total))
            })
            .ok_or_eyre("invalid header format")?;

        let start = start.parse::<u64>()?;
        let end = end.parse::<u64>()?;
        // NOTE total None represents "*" unknown global length
        let total = (total != "*").then(|| total.parse::<u64>()).transpose()?;

        Ok((RangeInclusive::new(start, end), total))
    }

    #[instrument(ret)]
    fn parse_unsatisfiable_content_range(value: &HeaderValue) -> eyre::Result<u64> {
        let value = value
            .to_str()?
            .strip_prefix("bytes */")
            .ok_or_eyre("expected unsatisfiable bytes range")?;

        let total = value.parse::<u64>()?;

        Ok(total)
    }

    #[instrument(ret)]
    fn parse_content_length(value: &HeaderValue) -> eyre::Result<u64> {
        value
            .to_str()
            .wrap_err("content length header is not UTF-8")
            .and_then(|str| str.parse().wrap_err("invalid content length value"))
    }
}

pub(crate) struct FileDownloadResponse {
    response: reqwest::Response,
    start: u64,
    total_length: Option<u64>,
}

impl FileDownloadResponse {
    async fn chunk(&mut self) -> eyre::Result<Option<Bytes>> {
        self.response.chunk().await.wrap_err("failed to read chunk")
    }

    #[instrument(skip_all)]
    pub(super) async fn write_chunks<W>(&mut self, mut writer: W) -> eyre::Result<()>
    where
        W: AsyncWrite + Unpin,
    {
        while let Some(bytes) = self
            .chunk()
            .await
            .wrap_err("error while getting response chunk")?
        {
            writer.write_all(&bytes).await?;
            writer.flush().await?;
        }

        Ok(())
    }

    pub(crate) fn start(&self) -> u64 {
        self.start
    }

    pub(crate) fn total_length(&self) -> Option<u64> {
        self.total_length
    }
}

#[derive(Debug, Clone)]
pub struct FileUploadResponse;

#[cfg(test)]
mod tests {
    use std::{io::Cursor, iter, ops::RangeInclusive};

    use httpmock::{
        Method::{GET, PUT},
        MockServer,
    };
    use reqwest::header::{HeaderMap, HeaderValue};
    use rstest::rstest;
    use tokio::io::{AsyncWrite, AsyncWriteExt};
    use tokio_stream::once;
    use url::Url;

    use crate::http::{FileDownloadResponse, FtHttpClient};

    async fn write_chunks<T>(mut response: FileDownloadResponse, mut buf: T)
    where
        T: AsyncWrite + AsyncWriteExt + Unpin,
    {
        while let Some(mut bytes) = response.chunk().await.unwrap() {
            buf.write_all_buf(&mut bytes).await.unwrap();
            buf.flush().await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_file_download() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-file")).unwrap();

        let binary_content = b"test data";
        let binary_size = binary_content.len();

        let mock_get_req = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/test-file")
                    // The full request doesn't need the range
                    .header_missing("range");
                then.status(reqwest::StatusCode::OK)
                    .header("Content-Length", "9")
                    .body(binary_content);
            })
            .await;

        let client = FtHttpClient::create().unwrap();
        let response = client
            .download(
                &file_url,
                HeaderMap::new(),
                0,
                Some(binary_size.try_into().unwrap()),
            )
            .await
            .unwrap();

        let mut output_buf = Vec::with_capacity(binary_size);
        write_chunks(response, Cursor::new(&mut output_buf)).await;

        assert_eq!(binary_content.as_slice(), output_buf.as_slice());

        mock_get_req.assert_async().await;
    }

    #[tokio::test]
    async fn ranged_download() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-file")).unwrap();

        let binary_content = [2u8; 8192];
        let binary_size = u64::try_from(binary_content.len()).unwrap();

        let client = FtHttpClient::create().unwrap();
        // Mocking a different range
        let range_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/test-file")
                    .header_includes("range", "bytes=4096-");
                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 4096-8191/8192")
                    .header("Content-Length", "4096")
                    .body(&binary_content[4096..]);
            })
            .await;

        // try download partial
        let mut output_buf = Vec::with_capacity(4096);
        let response = client
            .download(&file_url, HeaderMap::new(), 4096, Some(binary_size))
            .await
            .unwrap();
        write_chunks(response, Cursor::new(&mut output_buf)).await;
        assert_eq!(&binary_content[4096..], &output_buf);

        range_mock.assert_async().await;
    }

    #[tokio::test]
    async fn large_download() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-file")).unwrap();

        let binary_size: u64 = 60 * 1000 * 1000;
        let binary_content = vec![1u8; usize::try_from(binary_size).unwrap()];

        let complete_mock = server
            .mock_async(|when, then| {
                when.method(GET).path("/test-file").header_missing("range");

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Length", "60000000")
                    .body(&binary_content);
            })
            .await;

        let client = FtHttpClient::create().unwrap();
        let response = client
            .download(&file_url, HeaderMap::new(), 0, Some(binary_size))
            .await
            .unwrap();
        // try download complete
        let mut output_buf = Vec::with_capacity(binary_size.try_into().unwrap());
        write_chunks(response, Cursor::new(&mut output_buf)).await;
        // NOTE do not compare content, we are happy with the length being the same
        // assert_eq!(binary_content.as_slice(), output_buf.as_slice());
        assert_eq!(binary_content.len(), output_buf.len());
        complete_mock.assert_async().await;
    }

    #[rstest]
    #[case("bytes 42-1233/1234", (42..=1233, Some(1234)))]
    #[case("bytes 42-1233/*", (42..=1233, None))]
    fn parse_content_range(#[case] value: &str, #[case] cr: (RangeInclusive<u64>, Option<u64>)) {
        let value = HeaderValue::from_str(value).unwrap();

        let res = FtHttpClient::parse_content_range(&value).unwrap();

        assert_eq!(res, cr)
    }

    #[rstest]
    #[case::panic("none")]
    #[case::panic("bytes */47022")]
    fn parse_content_range_invalid(#[case] value: &str) {
        let value = HeaderValue::from_str(value).unwrap();

        let _ = FtHttpClient::parse_content_range(&value).unwrap_err();
    }

    #[rstest]
    #[case("bytes */47022", 47022)]
    fn parse_unsatisfiable_content_range(#[case] value: &str, #[case] exp: u64) {
        let value = HeaderValue::from_str(value).unwrap();

        let res = FtHttpClient::parse_unsatisfiable_content_range(&value).unwrap();

        assert_eq!(res, exp);
    }

    #[rstest]
    #[case::panic("bytes 42-1233/1234")]
    #[case::panic("bytes 42-1233/*")]
    #[case::panic("none")]
    fn parse_unsatisfiable_content_range_invalid(#[case] value: &str) {
        let value = HeaderValue::from_str(value).unwrap();

        let _ = FtHttpClient::parse_unsatisfiable_content_range(&value).unwrap_err();
    }

    #[tokio::test]
    async fn test_file_upload() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-upload")).unwrap();

        let binary_content = "test data";

        let mock_get_req = server
            .mock_async(|when, then| {
                when.method(PUT).path("/test-upload").body(binary_content);

                then.status(reqwest::StatusCode::OK);
            })
            .await;

        let body_stream: tokio_stream::Once<eyre::Result<_>> = once(Ok(binary_content));

        let client = FtHttpClient::create().unwrap();
        let _ = client
            .upload(&file_url, HeaderMap::new(), body_stream)
            .await
            .unwrap();

        mock_get_req.assert_async().await;
    }

    #[tokio::test]
    #[ignore = "slow"]
    async fn test_large_file_upload() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-upload")).unwrap();

        let size: usize = 60 * 1000 * 1000;
        let content: String = iter::repeat_n("a", size).collect();
        const CHUNK: &[u8] = &[b'a'; 60 * 1000];
        let needed_chunks = 1000;

        let mock_get_req = server
            .mock_async(|when, then| {
                when.method(PUT).path("/test-upload").body(&content);

                then.status(reqwest::StatusCode::OK);
            })
            .await;

        let body_stream: tokio_stream::Iter<_> =
            tokio_stream::iter(iter::repeat_n(CHUNK, needed_chunks).map(eyre::Ok));

        let client = FtHttpClient::create().unwrap();
        let _ = client
            .upload(&file_url, HeaderMap::new(), body_stream)
            .await
            .unwrap();

        mock_get_req.assert_async().await;
    }
}
