// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Clone, Parser)]
pub struct Cli {
    #[command(flatten)]
    pub astarte: AstarteConfig,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Send the data to astarte.
    Send {
        /// Token with access to app engine API to send data to Astarte.
        #[arg(long, env = "ASTARTE_TOKEN")]
        token: String,
        /// Specify the App engine api URLs.
        #[arg(long, env = "ASTARTE_API_URL")]
        appengine_url: String,
        /// Prints the requests as "curl" commands.
        #[arg(long, default_value = "false")]
        curl: bool,
        /// Path to a json file containing the data to send.
        data: PathBuf,
    },
    Receive,
}

#[derive(Debug, Clone, Args)]
pub struct AstarteConfig {
    /// Realm of the device.
    #[arg(long, env = "ASTARTE_REALM")]
    pub realm: String,
    /// Astarte device id.
    #[arg(long, env = "ASTARTE_DEVICE_ID")]
    pub device_id: String,
    /// Credential secret.
    #[arg(long, env = "ASTARTE_CREDENTIALS_SECRET")]
    pub credentials_secret: String,
    /// Astarte pairing url.
    #[arg(long, env = "ASTARTE_PAIRING_URL")]
    pub pairing_url: String,
    /// Astarte interfaces directory.
    #[arg(long, env = "ASTARTE_INTERFACES_DIR")]
    pub interfaces_dir: PathBuf,
    /// Astarte storage directory.
    #[arg(long, env = "ASTARTE_STORE_DIR")]
    pub store_dir: PathBuf,
}
