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

use log::{error, info};

use crate::error::DeviceManagerError;

pub fn reboot() -> Result<(), DeviceManagerError> {
    if std::env::var("DM_NO_REBOOT").is_ok() {
        info!("Dry run, exiting");

        std::process::exit(0);
    }

    // TODO: use systemd api
    let mut cmd = std::process::Command::new("shutdown");
    cmd.arg("-r");
    cmd.arg("now");

    let output = cmd.output()?;

    if output.status.success() && output.stderr.is_empty() {
        panic!("Reboot command was successful, bye");
    } else {
        error!("Reboot failed {:?}", output.stderr);
    }

    Ok(())
}
