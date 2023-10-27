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

use std::time::Duration;

use log::{debug, error, info};

use crate::error::DeviceManagerError;

pub async fn reboot() -> Result<(), DeviceManagerError> {
    debug!("waiting 5 secs before reboot");

    tokio::time::sleep(Duration::from_secs(5)).await;

    if std::env::var("DM_NO_REBOOT").is_ok() {
        info!("Dry run, exiting");

        std::process::exit(0);
    }

    // TODO: use systemd api
    let output = tokio::process::Command::new("shutdown")
        .args(["-r", "now"])
        .output()
        .await?;

    if output.status.success() && output.stderr.is_empty() {
        panic!("Reboot command was successful, bye");
    } else {
        error!("Reboot failed {:?}", output.stderr);
    }

    Ok(())
}
