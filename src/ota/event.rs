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

//! OTA event received from Astarte.

use std::ops::Deref;

use astarte_device_sdk::{types::TypeError, AstarteType, FromEvent};
use tracing::error;
use uuid::Uuid;

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(interface = "io.edgehog.devicemanager.OTARequest", path = "/request")]
pub struct OtaRequest {
    pub operation: OtaOperation,
    pub url: String,
    pub uuid: OtaUuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtaUuid(pub Uuid);

impl From<Uuid> for OtaUuid {
    fn from(value: Uuid) -> Self {
        OtaUuid(value)
    }
}

impl Deref for OtaUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TryFrom<AstarteType> for OtaUuid {
    type Error = TypeError;

    fn try_from(value: AstarteType) -> Result<Self, Self::Error> {
        let str = String::try_from(value)?;

        Uuid::try_parse(&str).map(OtaUuid).map_err(|err| {
            error!("coudln't parse Ota UUID: {}", stable_eyre::Report::new(err));

            TypeError::Conversion
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaOperation {
    Update,
    Cancel,
}

impl TryFrom<AstarteType> for OtaOperation {
    type Error = TypeError;

    fn try_from(value: AstarteType) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Update" => Ok(Self::Update),
            "Cancel" => Ok(Self::Cancel),
            _ => {
                error!("unrecognize Ota operation value {value}");

                Err(TypeError::Conversion)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::controller::event::RuntimeEvent;

    use super::*;

    use std::collections::HashMap;

    use astarte_device_sdk::{event::FromEventError, DeviceEvent, Value};

    #[test]
    fn should_convert_ota_from_event() {
        let operation = "Update";
        let url = "http://example.com";
        let uuid = Uuid::try_parse("04bf491c-af94-4e9d-813f-ebeebfb856a6").unwrap();

        let mut data = HashMap::new();
        data.insert("operation".to_string(), operation.into());
        data.insert("url".to_string(), url.into());
        data.insert("uuid".to_string(), uuid.to_string().into());

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object(data),
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Ota(OtaRequest {
                operation: OtaOperation::Update,
                url: url.to_string(),
                uuid: uuid.into(),
            })
        );
    }

    #[test]
    fn telemetry_missing_uuid() {
        let data = HashMap::from([(
            "url".to_string(),
            AstarteType::String("http://instance.ota.bin".to_string()),
        )]);

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object(data),
        })
        .unwrap_err();

        assert!(
            matches!(err, FromEventError::MissingField { .. }),
            "got err {err:?}"
        );
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_data_with_one_key() {
        let data = HashMap::from([
            (
                "url".to_string(),
                AstarteType::String("http://instance.ota.bin".to_string()),
            ),
            (
                "uuid".to_string(),
                AstarteType::String("bad_uuid".to_string()),
            ),
            (
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            ),
        ]);

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object(data),
        })
        .unwrap_err();

        assert!(matches!(err, FromEventError::Conversion { .. }));
    }

    #[tokio::test]
    async fn ota_event_fail_data_with_wrong_astarte_type() {
        let mut data = HashMap::new();
        data.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        data.insert("uuid".to_owned(), AstarteType::Integer(0));
        data.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object(data),
        })
        .unwrap_err();

        assert!(matches!(err, FromEventError::Conversion { .. }));
    }
}
