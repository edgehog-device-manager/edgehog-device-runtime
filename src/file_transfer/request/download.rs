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

//! File Transfer download request

use std::borrow::Cow;
use std::path::Path;
use std::time::Duration;

use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::{Context, OptionExt, eyre};
use minicbor::bytes::ByteVec;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use tracing::{instrument, warn};
use url::Url;
use uuid::Uuid;

use super::{Encoding, FileDigest, FileOptions, FilePermissions, conv_or_default};
use crate::file_transfer::interface::ServerToDevice;
use crate::file_transfer::interface::capabilities::{
    FILESYSTEM_TARGET, STORAGE_TARGET, STREAMING_TARGET,
};
use crate::file_transfer::interface::status::{FileTransferId, TransferDirection};
use crate::file_transfer::request::TransferJobTag;
use crate::jobs::derive;

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode)]
pub(crate) struct Download<'a> {
    #[cbor(skip)]
    pub(crate) id: Uuid,
    #[cbor(n(0), with = "derive::url")]
    pub(crate) url: Url,
    #[cbor(n(1), with = "derive::header_map")]
    pub(crate) headers: HeaderMap,
    #[n(2)]
    pub(crate) progress: bool,
    #[n(3)]
    pub(crate) digest_type: FileDigest,
    #[n(4)]
    pub(crate) digest: ByteVec,
    #[cbor(n(5), with = "derive::as_secs_opt")]
    pub(crate) ttl: Option<Duration>,
    #[n(6)]
    pub(crate) encoding: Option<Encoding>,
    #[n(7)]
    pub(crate) file_size: u64,
    #[n(8)]
    pub(crate) permission: FilePermissions,
    #[n(9)]
    pub(crate) destination: Destination<'a>,
}

impl<'a> Download<'a> {
    const SERIALIZED_VERSION: i32 = 0;

    // Returns the download file size.
    //
    // This will be none if compression is enabled since the file size is the uncompressed one.
    pub(crate) fn download_length(&self) -> Option<u64> {
        self.encoding.is_none().then_some(self.file_size)
    }

    pub(crate) fn transfer(&self) -> FileTransferId {
        FileTransferId {
            id: self.id,
            direction: TransferDirection::Download,
        }
    }
}

impl From<&Download<'_>> for FileOptions {
    fn from(value: &Download) -> Self {
        FileOptions {
            id: value.id,
            file_size: value.file_size,
            #[cfg(unix)]
            perm: value.permission,
            compression: value.encoding,
        }
    }
}

impl<'a> TryFrom<&'a ServerToDevice> for Download<'a> {
    type Error = eyre::Error;

    fn try_from(value: &'a ServerToDevice) -> Result<Self, Self::Error> {
        let ServerToDevice {
            id,
            url,
            http_header_keys,
            http_header_values,
            encoding: compression,
            file_size_bytes,
            progress,
            digest,
            ttl_seconds,
            file_mode,
            user_id,
            group_id,
            destination_type,
            destination,
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

        let ttl = conv_or_default(*ttl_seconds, 0)
            .wrap_err("couldn't convert ttl_seconds to duration")?
            .map(Duration::from_secs);

        let permission = FilePermissions::from_event(*file_mode, *user_id, *group_id)?;

        let file_size = u64::try_from(*file_size_bytes).wrap_err("couldn't convert file size")?;

        let (digest_type, digest) = digest
            .split_once(':')
            .ok_or_eyre("couldn't parse digest, missing ':' delimiter")?;

        let digest = hex::decode(digest)
            .map(ByteVec::from)
            .wrap_err("couldn't decode hex digest")?;

        let compression = (!compression.is_empty())
            .then(|| compression.parse())
            .transpose()?;

        let destination = Destination::from_str(destination_type, destination)?;

        Ok(Self {
            id: id.parse()?,
            url: url.parse()?,
            headers,
            encoding: compression,
            file_size,
            progress: *progress,
            digest_type: digest_type.parse()?,
            digest,
            ttl,
            permission,
            destination,
        })
    }
}

impl TryFrom<Download<'_>> for Job {
    type Error = eyre::Report;

    fn try_from(value: Download<'_>) -> Result<Self, Self::Error> {
        let data = minicbor::to_vec(&value).wrap_err("couldn't encode download request")?;

        Ok(Job {
            id: SqlUuid::new(value.id),
            job_type: JobType::FileTransfer,
            status: JobStatus::default(),
            version: Download::SERIALIZED_VERSION,
            tag: TransferJobTag::Download.into(),
            data,
            schedule_at: None,
        })
    }
}

impl TryFrom<Job> for Download<'_> {
    type Error = eyre::Report;

    fn try_from(value: Job) -> Result<Self, Self::Error> {
        let Job {
            id,
            job_type,
            status,
            version,
            tag,
            data,
            schedule_at,
        } = value;

        debug_assert_eq!(job_type, JobType::FileTransfer);
        debug_assert_eq!(status, JobStatus::InProgress);
        debug_assert_eq!(tag, i32::from(TransferJobTag::Download));
        debug_assert_eq!(version, Download::SERIALIZED_VERSION);
        debug_assert!(schedule_at.is_none());

        let mut this: Self =
            minicbor::decode(&data).wrap_err("couldn't decode download request")?;

        this.id = *id;

        Ok(this)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[repr(u8)]
pub(crate) enum Destination<'a> {
    #[n(0)]
    Storage,
    #[n(1)]
    Stream,
    #[n(2)]
    FileSystem {
        #[n(0)]
        path: Cow<'a, Path>,
    },
}

impl<'a> Destination<'a> {
    #[instrument]
    fn from_str(destination_type: &str, destination: &'a str) -> eyre::Result<Self> {
        match destination_type {
            STORAGE_TARGET => {
                if !destination.is_empty() {
                    warn!(destination, "storage destination should be empty");
                }

                Ok(Destination::Storage)
            }
            STREAMING_TARGET => {
                if !destination.is_empty() {
                    warn!(destination, "stream destination should be empty");
                }

                Ok(Destination::Stream)
            }
            FILESYSTEM_TARGET => Ok(Destination::FileSystem {
                path: Cow::Borrowed(Path::new(destination)),
            }),
            _ => Err(eyre!(
                "unrecognize file transfer target: {destination_type}",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::{Context, fixture, rstest};

    use crate::file_transfer::interface::tests::fs_server_to_device;
    use crate::tests::{Hexdump, with_insta};

    #[fixture]
    pub(crate) fn download_req() -> Download<'static> {
        Download {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            url: "https://s3.example.com".parse().unwrap(),
            headers: HeaderMap::from_iter([(
                AUTHORIZATION,
                HeaderValue::from_static(
                    "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==",
                ),
            )]),
            file_size: 4096,
            encoding: Some(Encoding::TarGz),
            progress: true,
            digest_type: FileDigest::Sha256,
            digest: hex::decode("28babb1cdf8aea6b62acc1097fdc83482cbf6e11c4fe7dcb39ae1682776baec5")
                .map(ByteVec::from)
                .unwrap(),
            ttl: None,
            permission: FilePermissions {
                mode: Some(544),
                user_id: Some(1000),
                group_id: Some(100),
            },
            destination: Destination::Storage,
        }
    }

    #[rstest]
    fn download_try_from_event(fs_server_to_device: ServerToDevice, download_req: Download) {
        let req = Download::try_from(&fs_server_to_device).unwrap();

        assert_eq!(req, download_req);
    }

    #[rstest]
    fn job_roundtrip(download_req: Download) {
        let mut job = Job::try_from(download_req.clone()).unwrap();
        let Job {
            id,
            job_type,
            status,
            version,
            tag,
            data,
            schedule_at,
        } = job.clone();

        assert_eq!(id, download_req.id.into());
        assert_eq!(job_type, JobType::FileTransfer);
        assert_eq!(status, JobStatus::Pending);
        assert_eq!(version, 0);
        assert_eq!(tag, i32::from(TransferJobTag::Download));
        assert!(schedule_at.is_none());

        job.status = JobStatus::InProgress;

        let res = Download::try_from(job).unwrap();

        assert_eq!(res, download_req);

        with_insta!({
            insta::assert_snapshot!(Hexdump(data));
        });
    }

    #[rstest]
    #[case(("storage", ""), Destination::Storage)]
    #[case(("streaming", ""), Destination::Stream)]
    #[case(("filesystem", "/foo/bar"), Destination::FileSystem { path: Path::new("/foo/bar").into() })]
    fn destination_roundtrip(
        #[context] ctx: Context,
        #[case] (destination_type, destination): (&str, &str),
        #[case] exp: Destination,
    ) {
        let res = Destination::from_str(destination_type, destination).unwrap();

        assert_eq!(res, exp);

        let buf = minicbor::to_vec(res).unwrap();

        let res: Destination = minicbor::decode(&buf).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, Hexdump(buf));
        });
    }
}
