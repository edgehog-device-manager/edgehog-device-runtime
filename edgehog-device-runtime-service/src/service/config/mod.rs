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

//! Service to dynamically configure the Device Runtime.

use std::path::PathBuf;

use edgehog_config::ConfigFile;
use tokio::sync::mpsc;

mod v1;

/// State for the configuration service.
#[derive(Debug)]
pub struct ConfigState {
    /// Directory to store the dynamic configurations into
    dir: PathBuf,
    /// Channel/buffer to send the received dynamic configurations.
    tx: mpsc::Sender<(PathBuf, ConfigFile)>,
}

impl ConfigState {
    /// Creates a new configuration service state
    pub fn new(dynamic_configs: PathBuf, tx: mpsc::Sender<(PathBuf, ConfigFile)>) -> Self {
        Self {
            dir: dynamic_configs,
            tx,
        }
    }
}
