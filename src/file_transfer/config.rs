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

use std::ops::Deref;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Max free space to keep on the device.
pub const DEFAULT_MAX_FREE_PERCENTAGE: Percentage = Percentage::new(20).unwrap();

/// Value from 0 to 100
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Percentage(u8);

impl Percentage {
    /// Creates a new percentage.
    pub const fn new(value: u8) -> Option<Self> {
        if value <= 100 {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Calculate the percentage.
    pub fn calculate(&self, value: u64) -> u64 {
        value.saturating_mul(self.0.into()).div_ceil(100)
    }
}

impl Deref for Percentage {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for Percentage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Percentage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;

        Self::new(value).ok_or(serde::de::Error::invalid_value(
            serde::de::Unexpected::Unsigned(value.into()),
            &"an Unsigned integer as percentage between 0 and 100",
        ))
    }
}

/// Configuration for the FileTransfer
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTransferConfig {
    /// Enables the file-transfer service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Sets a custom directory for the `storage` target.
    ///
    /// The default is the subdirectory `file-store/` in the configured `store_directory`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_dir: Option<PathBuf>,
    /// Percentage of free space reserved for the system.
    ///
    /// The default is the 20% of the free space can not bee occupied by a file transfer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_reserved: Option<Percentage>,
}

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
