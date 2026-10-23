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

//! Configuration for the file-transfer service.

use std::path::{Path, PathBuf};

use edgehog_config::file_transfer::{FileTransferConfig, Percentage};

/// Max free space to keep on the device.
pub const DEFAULT_MAX_FREE_PERCENTAGE: Percentage = Percentage::new(20).unwrap();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransferArgs {
    pub(crate) enabled: bool,
    pub(crate) storage_dir: PathBuf,
    pub(crate) storage_reserved: Percentage,
}

impl FileTransferArgs {
    /// Converts a configuration to the arguments to start the file transfer
    pub fn with_store_dir(config: Option<FileTransferConfig>, store_dir: &Path) -> Self {
        let config = config.unwrap_or_default();

        let enabled = config.enabled.unwrap_or(true);
        let storage_dir = config
            .storage_dir
            .unwrap_or_else(|| store_dir.join("file-store"));
        let storage_reserved = config
            .storage_reserved
            .unwrap_or(DEFAULT_MAX_FREE_PERCENTAGE);

        Self {
            enabled,
            storage_dir,
            storage_reserved,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::with_insta;

    use super::*;

    #[test]
    fn config_roundtrip() {
        let exp = FileTransferConfig {
            enabled: Some(true),
            storage_dir: Some(PathBuf::from("/foo/bar")),
            storage_reserved: Some(Percentage::new(42).unwrap()),
        };

        let toml_str = toml::to_string_pretty(&exp).unwrap();

        let res: FileTransferConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }
}
