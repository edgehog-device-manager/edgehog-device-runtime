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

use std::borrow::Cow;
use std::path::Path;

use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::{Context, eyre};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use tracing::{instrument, warn};
use url::Url;
use uuid::Uuid;

use super::{Encoding, TransferJobTag};
use crate::file_transfer::interface::DeviceToServer;
use crate::file_transfer::interface::capabilities::{
    FILESYSTEM_TARGET, STORAGE_TARGET, STREAMING_TARGET,
};
use crate::file_transfer::interface::status::{FileTransferId, TransferDirection};
use crate::jobs::derive;

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode)]
pub(crate) struct Upload<'a> {
    #[cbor(skip)]
    pub(crate) id: Uuid,
    #[cbor(n(0), with = "derive::url")]
    pub(crate) url: Url,
    #[cbor(n(1), with = "derive::header_map")]
    pub(crate) headers: HeaderMap,
    #[n(2)]
    pub(crate) progress: bool,
    #[n(3)]
    pub(crate) encoding: Option<Encoding>,
    #[n(4)]
    pub(crate) source: Source<'a>,
}

impl<'a> Upload<'a> {
    const SERIALIZED_VERSION: i32 = 0;

    pub(crate) fn transfer(&self) -> FileTransferId {
        FileTransferId {
            id: self.id,
            direction: TransferDirection::Upload,
        }
    }
}

impl<'a> TryFrom<&'a DeviceToServer> for Upload<'a> {
    type Error = eyre::Error;

    fn try_from(value: &'a DeviceToServer) -> Result<Self, Self::Error> {
        let DeviceToServer {
            id,
            url,
            http_header_keys,
            http_header_values,
            encoding: compression,
            progress,
            source_type,
            source,
        } = value;

        let headers = http_header_keys
            .iter()
            .zip(http_header_values)
            .map(|(k, v)| -> eyre::Result<(HeaderName, HeaderValue)> {
                let k = HeaderName::try_from(k)?;
                let mut v = HeaderValue::try_from(v)?;

                if k == AUTHORIZATION {
                    v.set_sensitive(true);
                }

                Ok((k, v))
            })
            .collect::<eyre::Result<HeaderMap>>()?;

        let compression = (!compression.is_empty())
            .then(|| compression.parse())
            .transpose()?;

        let source = Source::from_str(source_type, source)?;

        Ok(Self {
            id: id.parse()?,
            url: url.parse()?,
            headers,
            encoding: compression,
            progress: *progress,
            source,
        })
    }
}

impl TryFrom<Upload<'_>> for Job {
    type Error = eyre::Report;

    fn try_from(value: Upload) -> Result<Self, Self::Error> {
        let data = minicbor::to_vec(&value).wrap_err("couldn't encode upload request")?;

        Ok(Job {
            id: SqlUuid::new(value.id),
            job_type: JobType::FileTransfer,
            status: JobStatus::default(),
            version: Upload::SERIALIZED_VERSION,
            tag: TransferJobTag::Upload.into(),
            schedule_at: None,
            data,
        })
    }
}

impl TryFrom<Job> for Upload<'_> {
    type Error = eyre::Report;

    fn try_from(value: Job) -> Result<Self, Self::Error> {
        let Job {
            id,
            job_type,
            status: _,
            version,
            tag,
            data,
            schedule_at,
        } = value;

        debug_assert_eq!(job_type, JobType::FileTransfer);
        debug_assert_eq!(tag, i32::from(TransferJobTag::Upload));
        debug_assert_eq!(version, Upload::SERIALIZED_VERSION);
        debug_assert!(schedule_at.is_none());

        let mut this: Self = minicbor::decode(&data).wrap_err("couldn't decode upload request")?;
        this.id = id.into();

        Ok(this)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[repr(u8)]
pub(crate) enum Source<'a> {
    #[n(0)]
    Storage {
        #[cbor(n(0), with = "derive::uuid")]
        id: Uuid,
    },
    #[n(1)]
    Stream,
    #[n(2)]
    FileSystem {
        #[n(0)]
        path: Cow<'a, Path>,
    },
}

impl<'a> Source<'a> {
    #[instrument]
    fn from_str(source_type: &str, source: &'a str) -> eyre::Result<Self> {
        match source_type {
            STORAGE_TARGET => Ok(Self::Storage {
                id: source.parse()?,
            }),
            STREAMING_TARGET => {
                if !source.is_empty() {
                    warn!(source, "stream source should be empty");
                }

                Ok(Self::Stream)
            }
            FILESYSTEM_TARGET => Ok(Self::FileSystem {
                path: Cow::Borrowed(Path::new(source)),
            }),
            _ => Err(eyre!("unrecognize file transfer source: {source_type}",)),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use rstest::{Context, fixture, rstest};

    use crate::file_transfer::interface::tests::fs_device_to_server;
    use crate::tests::{Hexdump, with_insta};

    use super::*;

    #[fixture]
    pub(crate) fn upload_req() -> Upload<'static> {
        Upload {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            url: "https://s3.example.com".parse().unwrap(),
            headers: HeaderMap::from_iter([(
                AUTHORIZATION,
                HeaderValue::from_static(
                    "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==",
                ),
            )]),
            encoding: Some(Encoding::TarGz),
            progress: true,
            source: Source::Storage {
                id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            },
        }
    }

    #[rstest]
    fn upload_try_from_event(fs_device_to_server: DeviceToServer, upload_req: Upload) {
        let req = Upload::try_from(&fs_device_to_server).unwrap();

        assert_eq!(req, upload_req);
    }

    #[rstest]
    fn job_roundtrip(upload_req: Upload) {
        let job = Job::try_from(upload_req.clone()).unwrap();
        let Job {
            id,
            job_type,
            status,
            version,
            tag,
            data,
            schedule_at,
        } = job.clone();

        assert_eq!(id, upload_req.id.into());
        assert_eq!(job_type, JobType::FileTransfer);
        assert_eq!(status, JobStatus::Pending);
        assert_eq!(version, 0);
        assert_eq!(tag, i32::from(TransferJobTag::Upload));
        assert_eq!(tag, i32::from(TransferJobTag::Upload));
        assert!(schedule_at.is_none());

        let res = Upload::try_from(job).unwrap();

        assert_eq!(res, upload_req);

        with_insta!({
            insta::assert_snapshot!(Hexdump(data));
        });
    }

    #[rstest]
    #[case(Source::Storage{ id: "f904fa79-ecdf-4a9d-b3d4-9494a9014d1a".parse().unwrap() })]
    #[case(Source::Stream)]
    #[case(Source::FileSystem{ path: Cow::Borrowed(Path::new("/etc/config")) })]
    fn targets_roundtrip(#[context] ctx: Context, #[case] target: Source) {
        let buf = minicbor::to_vec(&target).unwrap();

        let res: Source = minicbor::decode(&buf).unwrap();

        assert_eq!(res, target);

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, Hexdump(buf));
        });
    }
}
