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

use edgehog_store::conversions::SqlUuid;
use edgehog_store::models::job::Job;
use edgehog_store::models::job::job_type::JobType;
use edgehog_store::models::job::status::JobStatus;
use eyre::Context;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use url::Url;
use uuid::Uuid;

use super::{Compression, JobTag, Target};
use crate::file_transfer::interface::DeviceToServer;
use crate::jobs::derive;

#[derive(Debug, Clone, PartialEq, minicbor::Encode, minicbor::Decode)]
pub(crate) struct Upload {
    #[cbor(skip)]
    pub(crate) id: Uuid,
    #[cbor(n(0), with = "derive::url")]
    pub(crate) url: Url,
    #[cbor(n(1), with = "derive::header_map")]
    pub(crate) headers: HeaderMap,
    #[n(2)]
    pub(crate) progress: bool,
    #[n(3)]
    pub(crate) compression: Option<Compression>,
    #[n(4)]
    pub(crate) source_type: Target,
    #[n(5)]
    pub(crate) source: String,
}

impl Upload {
    const SERIALIZED_VERSION: i32 = 0;
}

impl TryFrom<&DeviceToServer> for Upload {
    type Error = eyre::Error;

    fn try_from(value: &DeviceToServer) -> Result<Self, Self::Error> {
        let DeviceToServer {
            id,
            url,
            http_header_key,
            http_header_value,
            compression,
            progress,
            source_type,
            source,
        } = value;

        let headers = http_header_key
            .iter()
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

        let compression = (!compression.is_empty())
            .then(|| compression.parse())
            .transpose()?;

        Ok(Self {
            id: id.parse()?,
            url: url.parse()?,
            headers,
            compression,
            progress: *progress,
            source_type: source_type.parse()?,
            source: source.clone(),
        })
    }
}

impl TryFrom<&Upload> for Job {
    type Error = eyre::Report;

    fn try_from(value: &Upload) -> Result<Self, Self::Error> {
        let data = minicbor::to_vec(value).wrap_err("couldn't encode upload request")?;

        Ok(Job {
            id: SqlUuid::new(value.id),
            job_type: JobType::FileTransfer,
            status: JobStatus::default(),
            version: Upload::SERIALIZED_VERSION,
            tag: JobTag::Upload.into(),
            data,
        })
    }
}

impl TryFrom<Job> for Upload {
    type Error = eyre::Report;

    fn try_from(value: Job) -> Result<Self, Self::Error> {
        let Job {
            id,
            job_type,
            status: _,
            version,
            tag,
            data,
        } = value;

        debug_assert_eq!(job_type, JobType::FileTransfer);
        debug_assert_eq!(tag, i32::from(JobTag::Upload));
        debug_assert_eq!(version, Upload::SERIALIZED_VERSION);

        let mut this: Self = minicbor::decode(&data).wrap_err("couldn't decode upload request")?;
        this.id = id.into();

        Ok(this)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use rstest::{fixture, rstest};

    use crate::file_transfer::interface::tests::fs_device_to_server;
    use crate::tests::{Hexdump, with_insta};

    use super::*;

    #[fixture]
    pub(crate) fn upload_req() -> Upload {
        Upload {
            id: "6389218e-0e05-4587-96e3-3e6e2b522a2b".parse().unwrap(),
            url: "https://s3.example.com".parse().unwrap(),
            headers: HeaderMap::from_iter([(
                AUTHORIZATION,
                HeaderValue::from_static(
                    "Bearer tXYBVo1eA+8MTQTgFovzb9/nKej1d7zS4/k64l3Tm7tOkzxGemBJqDKN5lhEr1ARkb6AXpMqRc6FKo3kk800kA==",
                ),
            )]),
            compression: Some(Compression::TarGz),
            progress: true,
            source_type: Target::Storage,
            source: String::new(),
        }
    }

    #[rstest]
    fn upload_try_from_event(fs_device_to_server: DeviceToServer, upload_req: Upload) {
        let req = Upload::try_from(&fs_device_to_server).unwrap();

        assert_eq!(req, upload_req);
    }

    #[rstest]
    fn job_roundtrip(upload_req: Upload) {
        let job = Job::try_from(&upload_req).unwrap();
        let Job {
            id,
            job_type,
            status,
            version,
            tag,
            data,
        } = job.clone();

        assert_eq!(id, upload_req.id.into());
        assert_eq!(job_type, JobType::FileTransfer);
        assert_eq!(status, JobStatus::Pending);
        assert_eq!(version, 0);
        assert_eq!(tag, i32::from(JobTag::Upload));

        let res = Upload::try_from(job).unwrap();

        assert_eq!(res, upload_req);

        with_insta!({
            insta::assert_snapshot!(Hexdump(data));
        });
    }
}
