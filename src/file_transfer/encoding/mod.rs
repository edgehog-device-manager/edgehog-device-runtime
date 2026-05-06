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

//! Encoding Reader and Writer

use std::io;
use std::path::{Path, PathBuf};

use tracing::instrument;

pub(crate) mod tar_gz;

#[derive(Debug, Clone)]
pub(crate) enum Paths {
    File { base: PathBuf, file: PathBuf },
    Dir { base: PathBuf, dir: PathBuf },
}

impl Paths {
    #[instrument]
    pub(crate) async fn read(path: PathBuf) -> io::Result<Self> {
        let meta = tokio::fs::metadata(&path).await?;
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();

        if meta.is_dir() {
            Ok(Paths::Dir { base, dir: path })
        } else {
            Ok(Paths::File { base, file: path })
        }
    }
}
