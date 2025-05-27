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

use astarte_device_sdk::{types::TypeError, AstarteData, FromEvent};
use tracing::error;
use uuid::Uuid;

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(
    interface = "io.edgehog.devicemanager.OTARequest",
    path = "/request",
    aggregation = "object"
)]
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

impl TryFrom<AstarteData> for OtaUuid {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let str = String::try_from(value)?;

        Uuid::try_parse(&str).map(OtaUuid).map_err(|err| {
            error!(
                value = str,
                error = format!("{:#}", stable_eyre::Report::new(err)),
                "coudln't parse Ota UUID",
            );

            TypeError::Conversion {
                ctx: format!("couldn't parse Ota UUID: {str}"),
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtaOperation {
    Update,
    Cancel,
}

impl TryFrom<AstarteData> for OtaOperation {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Update" => Ok(Self::Update),
            "Cancel" => Ok(Self::Cancel),
            _ => {
                error!(value, "unrecognize Ota operation value");

                Err(TypeError::Conversion {
                    ctx: format!("unrecognize Ota operation value {value}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::controller::event::RuntimeEvent;

    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{event::FromEventError, DeviceEvent, Value};

    #[test]
    fn should_convert_ota_from_event() {
        let operation = "Update";
        let url = "http://example.com";
        let uuid = Uuid::try_parse("04bf491c-af94-4e9d-813f-ebeebfb856a6").unwrap();

        let data = AstarteObject::from_iter([
            ("operation".to_string(), operation.into()),
            ("url".to_string(), url.into()),
            ("uuid".to_string(), uuid.to_string().into()),
        ]);

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
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
        let data = AstarteObject::from_iter([(
            "url".to_string(),
            AstarteData::String("http://instance.ota.bin".to_string()),
        )]);

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
        })
        .unwrap_err();

        assert!(
            matches!(err, FromEventError::MissingField { .. }),
            "got err {err:?}"
        );
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_data_with_one_key() {
        let data = AstarteObject::from_iter([
            (
                "url".to_string(),
                AstarteData::String("http://instance.ota.bin".to_string()),
            ),
            (
                "uuid".to_string(),
                AstarteData::String("bad_uuid".to_string()),
            ),
            (
                "operation".to_string(),
                AstarteData::String("Update".to_string()),
            ),
        ]);

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
        })
        .unwrap_err();

        assert!(matches!(err, FromEventError::Conversion { .. }));
    }

    #[tokio::test]
    async fn ota_event_fail_data_with_wrong_astarte_type() {
        let data = AstarteObject::from_iter([
            (
                "url".to_owned(),
                AstarteData::String("http://ota.bin".to_owned()),
            ),
            ("uuid".to_owned(), AstarteData::Integer(0)),
            (
                "operation".to_string(),
                AstarteData::String("Update".to_string()),
            ),
        ]);

        let err = OtaRequest::from_event(DeviceEvent {
            interface: "io.edgehog.devicemanager.OTARequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
        })
        .unwrap_err();

        assert!(matches!(err, FromEventError::Conversion { .. }));
    }
}
