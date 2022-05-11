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
use std::process::Command;
use zbus::{dbus_interface, ConnectionBuilder};

#[derive(Debug, Parser)]
struct Cli {
    #[clap(short, long, required = true)]
    partition_path: String,
}

struct PartitionHandler {
    partition_path: String,
}

#[dbus_interface(name = "io.edgehog.PartitionHandler1")]
impl PartitionHandler {
    // Get boot consecutive fails
    fn get_consecutive_fail_boot(&self) -> String {
        let info_stdout = get_info_stdout(&self.partition_path);
        get_value_for_key_from_info(&info_stdout, "Consecutive fail boot")
    }

    // Get fail counter state
    fn get_boot_fail_counter_state(&self) -> String {
        let info_stdout = get_info_stdout(&self.partition_path);
        get_value_for_key_from_info(&info_stdout, "Boot fail counter state")
    }

    fn switch_partition(&self) -> bool {
        let result = Command::new("fw_sysdata")
            .args(["-t", "switchslot"])
            .arg("-s")
            .args(["-d", &self.partition_path])
            .status()
            .expect("Failed to execute fw_sysdata");

        result.success()
    }
}

fn get_info_stdout(partition_path: &str) -> String {
    String::from_utf8(
        Command::new("fw_sysdata")
            .args(["-t", "switchslot"])
            .arg("-p")
            .args(["-d", partition_path])
            .output()
            .expect("Failed to execute fw_sysdata")
            .stdout,
    )
    .expect("Failed to read output")
}

fn get_value_for_key_from_info(stdout: &str, key: &str) -> String {
    let stdout: Vec<String> = stdout
        .lines()
        .filter(|line| line.starts_with(key))
        .map(|line| line.split(":").nth(1).unwrap().to_owned())
        .collect();

    stdout.first().unwrap().trim().to_string()
}

// Simple DBUS service that exposes some info and the switch partition function of fw_sysdata
#[tokio::main]
async fn main() -> zbus::Result<()> {
    let Cli { partition_path } = Parser::parse();

    let partition_handler = PartitionHandler { partition_path };

    ConnectionBuilder::session()?
        .name("io.edgehog.PartitionHandler")?
        .serve_at("/io/edgehog/PartitionHandler", partition_handler)?
        .build()
        .await?;

    loop {
        std::thread::park()
    }
}

#[cfg(test)]
mod tests {
    use super::get_value_for_key_from_info;

    #[test]
    fn partition_handler_info_test() {
        let mock_output = "Switchslot - current state:
Boot from slot @: 00002
Boot from slot A: 00001
Current boot slot: B
Previous boot slot:
Consecutive fail boot: 0
Boot fail counter state: disabled";

        assert_eq!(
            get_value_for_key_from_info(mock_output, "Consecutive fail boot"),
            "0".to_string()
        );
        assert_eq!(
            get_value_for_key_from_info(mock_output, "Boot fail counter state"),
            "disabled".to_string()
        );
    }
}
