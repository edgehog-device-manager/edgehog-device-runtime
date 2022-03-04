/*
 * This file is part of Edgehog.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use clap::Parser;
use edgehog_device_runtime::DeviceManagerOptions;

#[derive(Debug, Parser)]
struct Cli {
    // Realm name
    #[clap(short, long)]
    realm: String,
    // Device id
    #[clap(short, long)]
    device_id: String,
    // Credentials secret
    #[clap(short, long)]
    credentials_secret: String,
    // Pairing URL
    #[clap(short, long)]
    pairing_url: String,
}

#[tokio::main]
async fn main() -> Result<(), edgehog_device_runtime::error::DeviceManagerError> {
    env_logger::init();

    let Cli {
        realm,
        device_id,
        credentials_secret,
        pairing_url,
    } = Parser::parse();

    let options = DeviceManagerOptions {
        realm,
        device_id,
        credentials_secret,
        pairing_url,
        interface_json_path: "./interfaces".to_string(),
    };

    let mut dm = edgehog_device_runtime::DeviceManager::new(options).await?;

    dm.init().await?;

    dm.run().await;

    Ok(())
}
