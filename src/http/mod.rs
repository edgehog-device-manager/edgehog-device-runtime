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

//! Handles an http client to download part of or an entire file.

use std::{fmt::Debug, ops::RangeInclusive};

use bytes::Bytes;
use eyre::{Context, OptionExt, Report, bail, eyre};
use reqwest::{
    StatusCode,
    header::{GetAll, HeaderMap, HeaderValue},
    redirect::Policy,
};
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

#[derive(Debug, thiserror::Error)]
pub enum FileTransferError {
    #[error("requested range can't be satisfied")]
    RangeNotSatisfiable(Option<(RangeInclusive<u64>, Option<u64>)>),
    #[error("expected range {exp:?} got {got:?}")]
    UnexpectedRange {
        exp: DownloadRange,
        got: DownloadRange,
    },
    #[error(transparent)]
    Other(#[from] Report),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRange {
    start: u64,
    total_len: u64,
}

impl DownloadRange {
    pub fn create(start: u64, total_len: u64) -> Option<Self> {
        (start < total_len).then_some(Self { start, total_len })
    }

    pub fn start(&self) -> u64 {
        self.start
    }

    pub fn total_len(&self) -> u64 {
        self.total_len
    }

    fn reset_start(mut self) -> Self {
        self.start = 0;

        self
    }
}

impl PartialOrd for DownloadRange {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (self.total_len == other.total_len).then(|| {
            // a range with lower start is bigger than one with a higher start
            self.start.cmp(&other.start).reverse()
        })
    }
}

impl std::fmt::Display for DownloadRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}..{}", self.start, self.total_len)
    }
}

/// Create the http client
#[derive(Debug)]
pub struct FileTransferHttpClient {
    client: reqwest::Client,
}

impl FileTransferHttpClient {
    pub fn create() -> Result<Self, FileTransferError> {
        let client = default_http_client_builder()
            .wrap_err("failed to build TLS config")?
            .build()
            .wrap_err("failed to build HTTP client")?;

        Ok(Self { client })
    }

    pub async fn download(
        &self,
        url: &Url,
        mut headers: HeaderMap,
        range: DownloadRange,
    ) -> eyre::Result<FileDownloadResponse> {
        headers.insert(
            reqwest::header::RANGE,
            HeaderValue::from_str(&format!("bytes={}-", range.start))
                .wrap_err("invalid range header value")?,
        );

        let response = self
            .client
            .get(url.as_str())
            .headers(headers.clone())
            .send()
            .await
            .wrap_err("get range request error")?;

        let check_res = Self::check_response(&response, range.clone());

        let (response, actual_range) = match check_res {
            Ok(range) => (response, range),
            Err(FileTransferError::UnexpectedRange { exp, got }) => {
                if got > exp {
                    (response, got)
                } else {
                    bail!("expected range '{exp}' received smaller range '{got}'");
                }
            }
            Err(FileTransferError::RangeNotSatisfiable(content_range)) => {
                error!(
                    ?content_range,
                    "the requested range can't be satisfied by the server, retrying full request"
                );

                if content_range
                    .and_then(|(_, total)| total)
                    .is_some_and(|total| total != range.total_len())
                {
                    bail!("the server states a different length than the one expected");
                }

                let response = self.retry_full_request(url, headers).await?;

                Self::check_full_response(&response, range.total_len())?;

                (response, range.reset_start())
            }
            Err(FileTransferError::Other(e)) => return Err(e),
        };

        Ok(FileDownloadResponse::new(response, actual_range))
    }

    async fn retry_full_request(
        &self,
        url: &Url,
        mut headers: HeaderMap,
    ) -> eyre::Result<reqwest::Response> {
        headers.remove(reqwest::header::RANGE);

        self.client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await
            .wrap_err("get request error")
    }

    fn check_full_response(response: &reqwest::Response, total_len: u64) -> eyre::Result<()> {
        let length = Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_LENGTH));

        match length {
            Ok(l) => {
                let server_len = Self::parse_content_length(l)
                    .ok_or_eyre("could not parse content length header")?;

                if server_len != total_len {
                    bail!("expected total length {total_len} got {server_len}");
                }
            }
            Err(err) => {
                warn!(%err, "content length not provided by the server");
            }
        }

        Ok(())
    }

    #[instrument(err)]
    fn check_response(
        response: &reqwest::Response,
        exp: DownloadRange,
    ) -> Result<DownloadRange, FileTransferError> {
        match response.status() {
            StatusCode::PARTIAL_CONTENT => trace!("received partial content status"),
            StatusCode::OK => {
                debug!("status ok, the response is not partial");

                Self::check_full_response(response, exp.total_len())?;

                // return the download range with a 0 start
                let exp = exp.reset_start();

                return Ok(exp);
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let content_range =
                    Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_RANGE))
                        .ok()
                        .map(|value| {
                            Self::parse_content_range(value).ok_or(eyre!("can't parse range value"))
                        })
                        .transpose()?;

                return Err(FileTransferError::RangeNotSatisfiable(content_range));
            }
            s => {
                return Err(FileTransferError::Other(eyre!(
                    "unexpected status code {s}"
                )));
            }
        }

        let value = Self::expect_one(response.headers().get_all(reqwest::header::CONTENT_RANGE))?;
        let (response_range, response_total) =
            Self::parse_content_range(value).ok_or(eyre!("can't parse range value"))?;

        let got = DownloadRange::create(
            *response_range.start(),
            response_range.end().saturating_add(1),
        )
        .ok_or_eyre("range received is malformed")?;

        if exp != got {
            return Err(FileTransferError::UnexpectedRange { exp, got });
        }

        if response_total.is_some_and(|response_total| response_total != exp.total_len) {
            warn!(
                response_total,
                expected = exp.total_len,
                "wrong response complete length but correct range"
            );
        }

        Ok(exp)
    }

    fn expect_one(values: GetAll<'_, HeaderValue>) -> eyre::Result<&HeaderValue> {
        if values
            .iter()
            .skip(1)
            .inspect(|v| error!(value = ?v, "unexpected header value"))
            .count()
            != 0
        {
            bail!("expected one value received more");
        }

        values
            .iter()
            .next()
            .ok_or_eyre("expected one value received none")
    }

    #[instrument(ret)]
    fn parse_content_range(value: &HeaderValue) -> Option<(RangeInclusive<u64>, Option<u64>)> {
        let value = value
            .to_str()
            .inspect_err(|_| error!(value = value.as_bytes(), "invalid string"))
            .ok()?;

        let Some(value) = value.strip_prefix("bytes ") else {
            error!(value, "expected bytes range got");
            return None;
        };

        let Some(((start, end), total)) = value.split_once('/').and_then(|(range, total)| {
            range
                .split_once('-')
                .map(|(start, end)| ((start, end), total))
        }) else {
            error!(value, "invalid header format");
            return None;
        };

        let start = start.parse::<u64>().ok()?;
        let end = end.parse::<u64>().ok()?;
        // NOTE total None represents "*" unknown global length
        let total = (total != "*")
            .then(|| total.parse::<u64>())
            .transpose()
            .inspect_err(|_| {
                error!(total, "can't parse total length");
            })
            .ok()?;

        Some((RangeInclusive::new(start, end), total))
    }

    #[instrument(ret)]
    fn parse_content_length(value: &HeaderValue) -> Option<u64> {
        let value = value
            .to_str()
            .inspect_err(|_| error!(value = value.as_bytes(), "invalid string"))
            .ok()?;

        let length = value.parse::<u64>().ok()?;

        Some(length)
    }
}

pub struct FileDownloadResponse {
    response: reqwest::Response,
    range: DownloadRange,
}

impl FileDownloadResponse {
    fn new(response: reqwest::Response, range: DownloadRange) -> Self {
        Self { response, range }
    }

    pub(super) async fn chunk(&mut self) -> Result<Option<Bytes>, FileTransferError> {
        self.response
            .chunk()
            .await
            .wrap_err("failed to read chunk")
            .map_err(FileTransferError::Other)
    }

    pub(super) fn range(&self) -> &DownloadRange {
        &self.range
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use httpmock::{Method::GET, MockServer};
    use reqwest::header::HeaderMap;
    use tokio::io::{AsyncWrite, AsyncWriteExt};
    use url::Url;

    use crate::http::{DownloadRange, FileDownloadResponse, FileTransferHttpClient};

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
                    .header_includes("range", "bytes=0-");
                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 0-8/9")
                    .body(binary_content);
            })
            .await;

        let client = FileTransferHttpClient::create().unwrap();
        let response = client
            .download(
                &file_url,
                HeaderMap::new(),
                DownloadRange::create(0, binary_size.try_into().unwrap()).unwrap(),
            )
            .await
            .unwrap();

        let mut output_buf = Vec::with_capacity(binary_size);
        write_chunks(response, Cursor::new(&mut output_buf)).await;

        assert_eq!(binary_content.as_slice(), output_buf.as_slice());

        mock_get_req.assert_async().await;
    }

    #[tokio::test]
    async fn test_ranged_download() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-file")).unwrap();

        let binary_content = [2u8; 8192];
        let binary_size = u64::try_from(binary_content.len()).unwrap();

        let complete_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/test-file")
                    .header_includes("range", "bytes=0-");

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 0-8191/8192")
                    .body(binary_content);
            })
            .await;

        let client = FileTransferHttpClient::create().unwrap();
        let response = client
            .download(
                &file_url,
                HeaderMap::new(),
                DownloadRange::create(0, binary_size).unwrap(),
            )
            .await
            .unwrap();
        // try download complete
        let mut output_buf = Vec::with_capacity(8192);
        write_chunks(response, Cursor::new(&mut output_buf)).await;
        assert_eq!(binary_content.as_slice(), output_buf.as_slice());
        complete_mock.assert_async().await;

        let client = FileTransferHttpClient::create().unwrap();
        // Mocking a different range
        let range_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/test-file")
                    .header_includes("range", "bytes=4096-");
                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 4096-8191/8192")
                    .body(&binary_content[4096..]);
            })
            .await;

        // try download partial
        let mut output_buf = Vec::with_capacity(4096);
        let response = client
            .download(
                &file_url,
                HeaderMap::new(),
                DownloadRange::create(4096, binary_size).unwrap(),
            )
            .await
            .unwrap();
        write_chunks(response, Cursor::new(&mut output_buf)).await;
        assert_eq!(&binary_content[4096..], &output_buf);

        range_mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_large_download() {
        let server = MockServer::start_async().await;
        let file_url = Url::parse(&server.url("/test-file")).unwrap();

        let binary_size: u64 = 60 * 1000 * 1000;
        let binary_content = vec![1u8; usize::try_from(binary_size).unwrap()];

        let complete_mock = server
            .mock_async(|when, then| {
                when.method(GET)
                    .path("/test-file")
                    .header_includes("range", "bytes=0-");

                then.status(reqwest::StatusCode::PARTIAL_CONTENT)
                    .header("Content-Range", "bytes 0-59999999/60000000")
                    .body(&binary_content);
            })
            .await;

        let client = FileTransferHttpClient::create().unwrap();
        let response = client
            .download(
                &file_url,
                HeaderMap::new(),
                DownloadRange::create(0, binary_size).unwrap(),
            )
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
}
