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

use edgehog_device_runtime::{error::DeviceManagerError, DeviceManagerOptions};
use log::info;

pub async fn read_options(
    override_config_file_path: Option<String>,
) -> Result<DeviceManagerOptions, DeviceManagerError> {
    let paths = ["edgehog-config.toml", "/etc/edgehog/config.toml"]
        .iter()
        .map(|f| f.to_string());

    let paths = override_config_file_path
        .into_iter()
        .chain(paths)
        .filter(|f| std::path::Path::new(f).exists());

    if let Some(path) = paths.into_iter().next() {
        info!("Found configuration file {path}");

        let config = tokio::fs::read_to_string(path).await?;

        let config = toml::from_str::<DeviceManagerOptions>(&config)?;

        Ok(config)
    } else {
        Err(DeviceManagerError::FatalError(
            "Configuration file not found".to_string(),
        ))
    }
}
