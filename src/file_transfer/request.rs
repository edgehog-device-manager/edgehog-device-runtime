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

use std::str::FromStr;
use std::time::Duration;

use eyre::{Context, eyre};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use url::Url;
use uuid::Uuid;

use super::interface::{DeviceToServer, ServerToDevice};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DownloadReq {
    id: Uuid,
    url: Url,
    headers: HeaderMap,
    progress: bool,
    digest: String,
    ttl: Option<Duration>,
    compression: Compression,
    permission: Permission,
    destination: Target,
}

impl TryFrom<ServerToDevice> for DownloadReq {
    type Error = eyre::Error;

    fn try_from(value: ServerToDevice) -> Result<Self, Self::Error> {
        let ServerToDevice {
            id,
            url,
            http_header_key,
            http_header_value,
            compression,
            progress,
            digest,
            ttl_seconds,
            file_mode,
            user_id,
            group_id,
            destination,
        } = value;

        let headers = http_header_key
            .into_iter()
            .zip(http_header_value)
            .map(|(k, v)| -> eyre::Result<(HeaderName, HeaderValue)> {
                let k = HeaderName::try_from(k)?;
                let mut v = HeaderValue::try_from(v)?;

                if k == AUTHORIZATION {
                    v.set_sensitive(true);
                }

                Ok((k, v))
            })
            .collect::<eyre::Result<HeaderMap>>()?;

        let ttl = conv_or_default(ttl_seconds, 0)
            .wrap_err("couldn't convert ttl_seconds to duration")?
            .map(Duration::from_secs);

        let permission = Permission::from_event(file_mode, user_id, group_id)?;

        Ok(Self {
            id: id.parse()?,
            url: url.parse()?,
            headers,
            compression: compression.parse()?,
            progress,
            digest,
            ttl,
            destination: destination.parse()?,
            permission,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UploadReq {
    id: Uuid,
    url: Url,
    headers: HeaderMap,
    progress: bool,
    digest: String,
    compression: Compression,
    source: Target,
}

impl TryFrom<DeviceToServer> for UploadReq {
    type Error = eyre::Error;

    fn try_from(value: DeviceToServer) -> Result<Self, Self::Error> {
        let DeviceToServer {
            id,
            url,
            http_header_key,
            http_header_value,
            compression,
            progress,
            digest,
            source,
        } = value;

        let headers = http_header_key
            .into_iter()
            .zip(http_header_value)
            .map(|(k, v)| -> eyre::Result<(HeaderName, HeaderValue)> {
                let k = HeaderName::try_from(k)?;
                let mut v = HeaderValue::try_from(v)?;

                if k == AUTHORIZATION {
                    v.set_sensitive(true);
                }

                Ok((k, v))
            })
            .collect::<eyre::Result<HeaderMap>>()?;

        Ok(Self {
            id: id.parse()?,
            url: url.parse()?,
            headers,
            compression: compression.parse()?,
            progress,
            digest,
            source: source.parse()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Target {
    Storage,
}

impl FromStr for Target {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "storage" => Ok(Target::Storage),
            _ => Err(eyre!("unrecognize file transfer target: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Compression {
    TarGz,
}

impl FromStr for Compression {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tar.gz" => Ok(Compression::TarGz),
            _ => Err(eyre!("unrecognize compression format: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Permission {
    file_mode: Option<u32>,
    user_id: Option<u32>,
    group_id: Option<u32>,
}

impl Permission {
    fn from_event(file_mode: i64, user_id: i64, group_id: i64) -> eyre::Result<Self> {
        let file_mode = conv_or_default(file_mode, 0).wrap_err("couldn't convert file mode")?;
        let user_id = conv_or_default(user_id, -1).wrap_err("couldn't convert user id")?;
        let group_id = conv_or_default(group_id, -1).wrap_err("couldn't convert group id")?;

        Ok(Self {
            file_mode,
            user_id,
            group_id,
        })
    }
}

fn conv_or_default<T, U>(value: T, default: T) -> Result<Option<U>, U::Error>
where
    T: PartialEq,
    U: TryFrom<T>,
{
    if value == default {
        Ok(None)
    } else {
        U::try_from(value).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use crate::file_transfer::interface::tests::{fs_device_to_server, fs_server_to_device};

    use super::*;

    #[fixture]
    pub(crate) fn download_req() -> DownloadReq {
        DownloadReq {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            url: "https://s3.example.com".parse().unwrap(),
            headers: HeaderMap::from_iter([(
                AUTHORIZATION,
                HeaderValue::from_static(
                    "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==",
                ),
            )]),
            compression: Compression::TarGz,
            progress: true,
            digest: "sha256:28babb1cdf8aea6b62acc1097fdc83482cbf6e11c4fe7dcb39ae1682776baec5"
                .to_string(),
            ttl: None,
            permission: Permission {
                file_mode: Some(544),
                user_id: Some(1000),
                group_id: Some(100),
            },
            destination: Target::Storage,
        }
    }

    #[fixture]
    pub(crate) fn upload_req() -> UploadReq {
        UploadReq {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            url: "https://s3.example.com".parse().unwrap(),
            headers: HeaderMap::from_iter([(
                AUTHORIZATION,
                HeaderValue::from_static(
                    "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==",
                ),
            )]),
            compression: Compression::TarGz,
            progress: true,
            digest: "sha256:28babb1cdf8aea6b62acc1097fdc83482cbf6e11c4fe7dcb39ae1682776baec5"
                .to_string(),
            source: Target::Storage,
        }
    }

    #[rstest]
    fn download_try_from_event(fs_server_to_device: ServerToDevice, download_req: DownloadReq) {
        let req = DownloadReq::try_from(fs_server_to_device).unwrap();

        assert_eq!(req, download_req);
    }

    #[rstest]
    fn upload_try_from_event(fs_device_to_server: DeviceToServer, upload_req: UploadReq) {
        let req = UploadReq::try_from(fs_device_to_server).unwrap();

        assert_eq!(req, upload_req);
    }
}
