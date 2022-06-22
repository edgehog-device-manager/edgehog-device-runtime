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
use procfs::cmdline;
use std::fs;
use uuid::Uuid;
use zbus::{dbus_interface, ConnectionBuilder};

const DMI_SERIAL_FILE_PATH: &str = "/sys/class/dmi/id/board_serial";
const DEFAULT_NAMESPACE: &str = "f79ad91f-c638-4889-ae74-9d001a3b4cf8";

#[derive(Debug, Parser)]
struct Cli {
    // Retrieve hardware id from file
    #[clap(short, long, conflicts_with_all=&["use-dmi-serial","kernel-cmdline-key"])]
    file_path: Option<String>,

    // Shortcut per file-path = "/sys/class/dmi/id/board_serial"
    #[clap(short, long, takes_value = false, conflicts_with_all=&["file-path","kernel-cmdline-key"])]
    use_dmi_serial : bool,

    // Retrieve hardware id from Kernel parameters in the form key=value
    #[clap(short, long, conflicts_with_all=&["use-dmi-serial","file-path"])]
    kernel_cmdline_key: Option<String>,
}

struct Device {
    file_path: Option<String>,
    kernel_cmdline_key: Option<String>,
}

#[dbus_interface(name = "io.edgehog.Device1")]
impl Device {
    // Get hardware id using starting parameter.
    fn get_hardware_id(&self, namespace: &str) -> String {
        let mut data: String = "".to_string();
        if self.file_path.is_some() {
            data = fs::read_to_string(&self.file_path.clone().unwrap()).unwrap_or_default();
        }
        if self.kernel_cmdline_key.is_some() {
            let cmdline_params = cmdline().unwrap();
            for param in cmdline_params.iter() {
                if param.starts_with(&self.kernel_cmdline_key.clone().unwrap()) {
                    let first_half = format!("{}=", &self.kernel_cmdline_key.clone().unwrap());
                    data = param.replace(&first_half, "");
                }
            }
        }
        let ns = if namespace.is_empty() {
            DEFAULT_NAMESPACE
        } else {
            namespace
        };

        let namespace = Uuid::parse_str(ns).unwrap();
        let uuid = Uuid::new_v5(&namespace, data.trim().as_bytes());
        base64::encode_config(uuid.as_bytes(), base64::URL_SAFE_NO_PAD)
    }
}

// Simple DBUS service that retrieves a machine specific id and publish it on a system channel
#[tokio::main]
async fn main() -> zbus::Result<()> {
    let Cli {
        file_path,
        use_dmi_serial ,
        kernel_cmdline_key,
    } = Parser::parse();

    if file_path.is_none() && kernel_cmdline_key.is_none() && !use_dmi_serial  {
        let error_msg = "One parameter must be provided".to_string();
        return Err(zbus::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            error_msg,
        )));
    }

    let device = if use_dmi_serial  {
        Device {
            file_path: Some(DMI_SERIAL_FILE_PATH.to_string()),
            kernel_cmdline_key,
        }
    } else {
        Device {
            file_path,
            kernel_cmdline_key,
        }
    };

    ConnectionBuilder::system()?
        .name("io.edgehog.Device")?
        .serve_at("/io/edgehog/Device", device)?
        .build()
        .await?;

    loop {
        std::thread::park()
    }
}
