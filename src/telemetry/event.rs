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

//! Telemetry event from Astarte.

use std::time::Duration;

use astarte_device_sdk::{
    event::FromEventError, types::TypeError, AstarteType, DeviceEvent, FromEvent,
};
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryEvent {
    pub interface: String,
    pub config: TelemetryConfig,
}

impl FromEvent for TelemetryEvent {
    type Err = FromEventError;

    fn from_event(event: DeviceEvent) -> Result<Self, Self::Err> {
        let interface = TelemetryConfig::interface_from_path(&event.path).ok_or_else(|| {
            FromEventError::Path {
                interface: "io.edgehog.devicemanager.config.Telemetry",
                base_path: event.path.clone(),
            }
        })?;

        TelemetryConfig::from_event(event).map(|config| TelemetryEvent { interface, config })
    }
}

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(
    interface = "io.edgehog.devicemanager.config.Telemetry",
    aggregation = "individual"
)]
pub enum TelemetryConfig {
    #[mapping(endpoint = "/request/%{interface_name}/enable", allow_unset = true)]
    Enable(Option<bool>),
    #[mapping(
        endpoint = "/request/%{interface_name}/periodSeconds",
        allow_unset = true
    )]
    Period(Option<TelemetryPeriod>),
}

impl TelemetryConfig {
    fn interface_from_path(path: &str) -> Option<String> {
        path.strip_prefix('/')
            .and_then(|path| path.split('/').nth(1))
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryPeriod(pub Duration);

impl TryFrom<AstarteType> for TelemetryPeriod {
    type Error = TypeError;

    fn try_from(value: AstarteType) -> Result<Self, Self::Error> {
        let secs = i64::try_from(value).map(|i| match u64::try_from(i) {
            Ok(secs) => secs,
            Err(_) => {
                warn!("Telemetry period seconds value too big {i}, capping to u64::MAX");

                u64::MAX
            }
        })?;

        Ok(Self(Duration::from_secs(secs)))
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::Value;

    use crate::controller::event::RuntimeEvent;

    use super::*;

    #[test]
    fn should_convert_telemetry_from_event() {
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.config.Telemetry".to_string(),
            path: "/request/foo/enable".to_string(),
            data: Value::Individual(true.into()),
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Telemetry(TelemetryEvent {
                interface: "foo".to_string(),
                config: TelemetryConfig::Enable(Some(true))
            })
        );

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.config.Telemetry".to_string(),
            path: "/request/foo/periodSeconds".to_string(),
            data: Value::Individual(AstarteType::LongInteger(42)),
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Telemetry(TelemetryEvent {
                interface: "foo".to_string(),
                config: TelemetryConfig::Period(Some(TelemetryPeriod(Duration::from_secs(42))))
            })
        );
    }
}
