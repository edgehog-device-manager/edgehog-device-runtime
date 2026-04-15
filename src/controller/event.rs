// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

use std::fmt::Display;

use astarte_device_sdk::{DeviceEvent, FromEvent, event::FromEventError};

use crate::{commands::Commands, telemetry::event::TelemetryEvent};

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    Command(Commands),
    Telemetry(TelemetryEvent),
    #[cfg(feature = "file-transfer")]
    FileTransfer(crate::file_transfer::interface::request::FileTransferRequest),
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    Ota(crate::ota::event::OtaRequest),
    #[cfg(all(feature = "zbus", target_os = "linux"))]
    Led(crate::led_behavior::LedEvent),
    #[cfg(feature = "containers")]
    Container(Box<edgehog_containers::requests::ContainerRequest>),
    #[cfg(feature = "forwarder")]
    Forwarder(edgehog_forwarder::astarte::SessionInfo),
}

impl Display for RuntimeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeEvent::Command(_commands) => {
                write!(f, "Command")
            }
            RuntimeEvent::Telemetry(_telemetry_event) => {
                write!(f, "Telemetry")
            }
            #[cfg(feature = "file-transfer")]
            RuntimeEvent::FileTransfer(_file_transfer_event) => {
                write!(f, "FileTransfer")
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            RuntimeEvent::Ota(_ota_request) => {
                write!(f, "Ota")
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            RuntimeEvent::Led(_led_event) => {
                write!(f, "Led")
            }
            #[cfg(feature = "containers")]
            RuntimeEvent::Container(_container_request) => {
                write!(f, "Container")
            }
            #[cfg(feature = "forwarder")]
            RuntimeEvent::Forwarder(_session_info) => {
                write!(f, "Forwarder")
            }
        }
    }
}

impl FromEvent for RuntimeEvent {
    type Err = FromEventError;

    fn from_event(event: DeviceEvent) -> Result<Self, Self::Err> {
        match event.interface.as_str() {
            "io.edgehog.devicemanager.Commands" => {
                Commands::from_event(event).map(RuntimeEvent::Command)
            }
            "io.edgehog.devicemanager.config.Telemetry" => {
                TelemetryEvent::from_event(event).map(RuntimeEvent::Telemetry)
            }
            #[cfg(feature = "file-transfer")]
            interface if interface.starts_with("io.edgehog.devicemanager.fileTransfer") => {
                crate::file_transfer::interface::request::FileTransferRequest::from_event(event)
                    .map(RuntimeEvent::FileTransfer)
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            "io.edgehog.devicemanager.LedBehavior" => {
                crate::led_behavior::LedEvent::from_event(event).map(RuntimeEvent::Led)
            }
            #[cfg(all(feature = "zbus", target_os = "linux"))]
            "io.edgehog.devicemanager.OTARequest" => {
                crate::ota::event::OtaRequest::from_event(event).map(RuntimeEvent::Ota)
            }
            #[cfg(feature = "containers")]
            interface if interface.starts_with("io.edgehog.devicemanager.apps") => {
                edgehog_containers::requests::ContainerRequest::from_event(event)
                    .map(|event| RuntimeEvent::Container(Box::new(event)))
            }
            #[cfg(feature = "forwarder")]
            "io.edgehog.devicemanager.ForwarderSessionRequest" => {
                edgehog_forwarder::astarte::SessionInfo::from_event(event)
                    .map(RuntimeEvent::Forwarder)
            }
            _ => Err(FromEventError::Interface(event.interface)),
        }
    }
}
