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

use astarte_device_sdk::{types::TypeError, AstarteData, FromEvent};
use tracing::error;

use crate::error::DeviceManagerError;

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(
    interface = "io.edgehog.devicemanager.Commands",
    aggregation = "individual"
)]
pub enum Commands {
    #[mapping(endpoint = "/request")]
    Request(CmdReq),
}

impl Commands {
    /// Returns `true` if the cmd req is [`Reboot`].
    ///
    /// [`Reboot`]: CmdReq::Reboot
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    #[must_use]
    pub fn is_reboot(&self) -> bool {
        matches!(self, Self::Request(CmdReq::Reboot))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmdReq {
    Reboot,
}

impl TryFrom<AstarteData> for CmdReq {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Reboot" => Ok(CmdReq::Reboot),
            _ => {
                error!("unrecognize Commands request value {value}");

                Err(TypeError::Conversion {
                    ctx: format!("unrecognize Commands request value {value}"),
                })
            }
        }
    }
}

/// handle io.edgehog.devicemanager.Commands
pub(crate) async fn execute_command(cmd: Commands) -> Result<(), DeviceManagerError> {
    match cmd {
        Commands::Request(CmdReq::Reboot) => crate::power_management::reboot().await,
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{DeviceEvent, Value};

    use crate::controller::event::RuntimeEvent;

    use super::*;

    #[test]
    fn should_convert_command_from_event() {
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.Commands".to_string(),
            path: "/request".to_string(),
            data: Value::Individual {
                data: "Reboot".into(),
                timestamp: Utc::now(),
            },
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Command(Commands::Request(CmdReq::Reboot))
        );
    }
}
