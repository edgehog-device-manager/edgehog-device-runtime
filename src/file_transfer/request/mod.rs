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

use std::str::FromStr;

use edgehog_store::models::job::Job;
use eyre::{Context, bail, eyre};
use uuid::Uuid;

use crate::file_transfer::interface::capabilities::{STORAGE_TARGET, STREAMING_TARGET, TAR_GZ};

use self::download::Download;
use self::upload::Upload;

use super::FileOptions;

pub(crate) mod download;
pub(crate) mod upload;

#[derive(Debug)]
pub(crate) enum Request {
    Download(Download),
    Upload(Upload),
}

impl Request {
    pub(crate) fn id(&self) -> &Uuid {
        match self {
            Request::Download(download) => &download.id,
            Request::Upload(upload) => &upload.id,
        }
    }
}

impl TryFrom<&Request> for Job {
    type Error = eyre::Report;

    fn try_from(value: &Request) -> Result<Self, Self::Error> {
        match value {
            Request::Download(download_req) => Job::try_from(download_req),
            Request::Upload(upload_req) => Job::try_from(upload_req),
        }
    }
}

impl TryFrom<Job> for Request {
    type Error = eyre::Report;

    fn try_from(value: Job) -> Result<Self, Self::Error> {
        match JobTag::try_from(value.tag)? {
            JobTag::Download => Download::try_from(value).map(Request::Download),
            JobTag::Upload => Upload::try_from(value).map(Request::Upload),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum JobTag {
    Download = 0,
    Upload = 1,
}

impl From<JobTag> for i32 {
    fn from(value: JobTag) -> Self {
        value as i32
    }
}

impl TryFrom<i32> for JobTag {
    type Error = eyre::Report;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(JobTag::Download),
            1 => Ok(JobTag::Upload),
            _ => bail!("unrecognize file transfer job tag {value}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(index_only)]
#[repr(u8)]
pub(crate) enum Target {
    #[n(0)]
    Storage = 0,
    #[n(1)]
    Stream = 1,
}

impl From<Target> for u8 {
    fn from(value: Target) -> Self {
        value as u8
    }
}

impl FromStr for Target {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            STORAGE_TARGET => Ok(Target::Storage),
            STREAMING_TARGET => Ok(Target::Stream),
            _ => Err(eyre!("unrecognize file transfer target: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(index_only)]
#[repr(u8)]
pub(crate) enum Encoding {
    #[n(0)]
    TarGz = 0,
}

impl From<Encoding> for u8 {
    fn from(value: Encoding) -> Self {
        value as u8
    }
}

impl FromStr for Encoding {
    type Err = eyre::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TAR_GZ => Ok(Encoding::TarGz),
            _ => Err(eyre!("unrecognize compression format: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, minicbor::Encode, minicbor::Decode)]
pub(crate) struct FilePermissions {
    #[n(0)]
    pub(crate) mode: Option<u32>,
    #[n(1)]
    pub(crate) user_id: Option<u32>,
    #[n(2)]
    pub(crate) group_id: Option<u32>,
}

impl FilePermissions {
    fn from_event(file_mode: i64, user_id: i64, group_id: i64) -> eyre::Result<Self> {
        let file_mode = conv_or_default(file_mode, 0).wrap_err("couldn't convert file mode")?;
        let user_id = conv_or_default(user_id, -1).wrap_err("couldn't convert user id")?;
        let group_id = conv_or_default(group_id, -1).wrap_err("couldn't convert group id")?;

        Ok(Self {
            mode: file_mode,
            user_id,
            group_id,
        })
    }

    #[cfg(unix)]
    pub(super) fn mode(&self) -> u32 {
        self.mode.unwrap_or(0o600)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(index_only)]
#[repr(u8)]
pub(crate) enum FileDigest {
    #[n(0)]
    Sha256 = 0,
}

impl From<FileDigest> for u8 {
    fn from(value: FileDigest) -> Self {
        value as u8
    }
}

impl From<FileDigest> for aws_lc_rs::digest::Context {
    fn from(value: FileDigest) -> Self {
        match value {
            FileDigest::Sha256 => aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256),
        }
    }
}

impl FromStr for FileDigest {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sha256" => Ok(FileDigest::Sha256),
            _ => Err(eyre!("unrecognize file digest: {s}")),
        }
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
    use rstest::{Context, rstest};

    use super::*;
    use crate::tests::{Hexdump, with_insta};

    impl Request {
        pub(crate) fn target(&self) -> &String {
            match self {
                Request::Download(download) => &download.destination,
                Request::Upload(upload) => &upload.source,
            }
        }
    }

    #[rstest]
    #[case("storage", Target::Storage)]
    #[case("streaming", Target::Stream)]
    fn targets_roundtrip(#[context] ctx: Context, #[case] input: &str, #[case] exp: Target) {
        let res: Target = input.parse().unwrap();

        assert_eq!(res, exp);

        let buf = minicbor::to_vec(res).unwrap();

        let res: Target = minicbor::decode(&buf).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            let name = ctx.case.unwrap().to_string();

            insta::assert_snapshot!(name, Hexdump(buf));
        });
    }

    #[rstest]
    #[case("tar.gz", Encoding::TarGz)]
    fn encoding_roundtrip(#[context] ctx: Context, #[case] input: &str, #[case] exp: Encoding) {
        let res: Encoding = input.parse().unwrap();

        assert_eq!(res, exp);

        let buf = minicbor::to_vec(res).unwrap();

        let res: Encoding = minicbor::decode(&buf).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            let name = ctx.case.unwrap().to_string();

            insta::assert_snapshot!(name, Hexdump(buf));
        });
    }
}
