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

//! Stream the bytes to a named pipe or unix socket.

use tokio::io::{AsyncRead, AsyncWrite};
use tracing::instrument;
use uuid::Uuid;

use super::FileOptions;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        pub(crate) type SysPipe = self::unix::MakeFifo;
    } else if #[cfg(windows)] {
        pub(crate) type SysPipe = self::windows::MakeNamedPipe;
    }
}

#[derive(Debug)]
pub(crate) struct Streaming<S> {
    sys: S,
}

impl<S> Streaming<S> {
    #[instrument(skip_all, fields(id = %opt.id))]
    pub(crate) async fn open_writer(
        &self,
        opt: &FileOptions,
    ) -> eyre::Result<(S::Writer, aws_lc_rs::digest::Context)>
    where
        S: Pipe,
    {
        let writer = self.sys.open_writer(opt).await?;
        let digest = aws_lc_rs::digest::Context::from(opt.file_digest);

        Ok((writer, digest))
    }

    #[instrument(skip(self))]
    pub(crate) async fn create_reader(&self, id: &Uuid) -> eyre::Result<S::Reader>
    where
        S: Pipe,
    {
        let reader = self.sys.create_reader(id).await?;

        Ok(reader)
    }
}

impl Streaming<SysPipe> {
    pub(crate) fn with_sys() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                Self {
                    sys: self::unix::MakeFifo::new(),
                }
            } else if #[cfg(windows)] {
                Self {
                    sys: self::windows::MakeNamedPipe::new(),
                }
            }
        }
    }
}

pub(crate) trait Pipe {
    type Reader: AsyncRead;
    type Writer: AsyncWrite;

    fn open_writer(
        &self,
        opt: &FileOptions,
    ) -> impl Future<Output = eyre::Result<Self::Writer>> + Send;

    fn create_reader(&self, id: &Uuid) -> impl Future<Output = eyre::Result<Self::Reader>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_dir() {
        Streaming::with_sys();
    }
}
