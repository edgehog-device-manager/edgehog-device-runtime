// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Implement the interaction with the [Astarte rust SDK](astarte_device_sdk).
//!
//! Module responsible for handling a connection between a Device and Astarte.

use std::hash::Hash;

use astarte_device_sdk::{Error as SdkError, FromEvent, IntoAstarteObject};
use url::{ParseError, Url};

/// Astarte errors.
#[non_exhaustive]
#[derive(displaydoc::Display, thiserror::Error, Debug)]
pub enum AstarteError {
    /// Error occurring when different fields from those of the mapping are received.
    Sdk(#[from] SdkError),

    /// Missing session token.
    MissingSessionToken,

    /// Error while parsing an url, `{0}`.
    ParseUrl(#[from] ParseError),
}

/// Struct representing the fields of an aggregated object the Astarte server can send to the device.
#[derive(Debug, Clone, Eq, PartialEq, Hash, FromEvent, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.ForwarderSessionRequest",
    path = "/request",
    aggregation = "object"
)]
pub struct SessionInfo {
    /// Hostname or IP address.
    pub host: String,
    /// Port number.
    pub port: i32,
    /// Session token.
    pub session_token: String,
    /// Flag to enable secure session establishment
    pub secure: bool,
}

impl TryFrom<&SessionInfo> for Url {
    type Error = AstarteError;

    fn try_from(value: &SessionInfo) -> Result<Self, Self::Error> {
        if value.session_token.is_empty() {
            return Err(AstarteError::MissingSessionToken);
        }

        let schema = if value.secure { "wss" } else { "ws" };

        Url::parse_with_params(
            &format!(
                "{}://{}:{}/device/websocket",
                schema, value.host, value.port
            ),
            &[("session", &value.session_token)],
        )
        .map_err(AstarteError::ParseUrl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::types::AstarteData;
    use astarte_device_sdk::{DeviceEvent, Value};
    use std::net::Ipv4Addr;
    use url::Host;

    fn create_sinfo(session_token: &str) -> SessionInfo {
        SessionInfo {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: 8080,
            session_token: session_token.to_string(),
            secure: false,
        }
    }

    fn create_astarte_event(
        host: &str,
        port: i32,
        session_token: &str,
        secure: bool,
    ) -> DeviceEvent {
        let mut data = AstarteObject::new();

        data.insert("host".to_string(), AstarteData::String(host.to_string()));
        data.insert("port".to_string(), AstarteData::Integer(port));
        data.insert(
            "session_token".to_string(),
            AstarteData::String(session_token.to_string()),
        );
        data.insert("secure".to_string(), AstarteData::Boolean(secure));

        DeviceEvent {
            interface: "io.edgehog.devicemanager.ForwarderSessionRequest".to_string(),
            path: "/request".to_string(),
            data: Value::Object {
                data,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn test_astarte_aggregate() {
        let sinfo = create_sinfo("test_token");

        let expected = [
            ("host", AstarteData::String("127.0.0.1".to_string())),
            ("port", AstarteData::Integer(8080)),
            (
                "session_token",
                AstarteData::String("test_token".to_string()),
            ),
        ];

        let res: Result<AstarteObject, astarte_device_sdk::Error> = sinfo.try_into();

        assert!(res.is_ok());

        let res = res.unwrap();

        for (key, exp_val) in expected {
            assert_eq!(*res.get(key).unwrap(), exp_val);
        }
    }

    #[test]
    fn test_try_from_sinfo() {
        // empty session token generates error
        let mut sinfo = create_sinfo("");

        assert!(Url::try_from(&sinfo).is_err());

        // ok
        sinfo = create_sinfo("test_token");

        let case = Url::try_from(&sinfo).unwrap();

        assert_eq!(case.host(), Some(Host::Ipv4(Ipv4Addr::LOCALHOST)));
        assert_eq!(case.port(), Some(8080));
        assert_eq!(case.query(), Some("session=test_token"));
    }

    #[test]
    fn test_retrieve_sinfo() {
        let err_cases = [
            create_astarte_event("", 8080, "test_token", false),
            create_astarte_event("127.0.0.1", -1, "test_token", false),
            create_astarte_event("127.0.0.1", 8080, "", false),
        ];

        for event in err_cases {
            let sinfo = SessionInfo::from_event(event).unwrap();
            assert!(Url::try_from(&sinfo).is_err());
        }

        let event = create_astarte_event("127.0.0.1", 8080, "test_token", false);
        let sinfo = SessionInfo::from_event(event).unwrap();

        assert_eq!(sinfo.host, Ipv4Addr::LOCALHOST.to_string());
        assert_eq!(sinfo.port, 8080);
        assert_eq!(sinfo.session_token, "test_token".to_string());
        assert!(!sinfo.secure);

        let url = Url::try_from(&sinfo).unwrap();
        let exp = Url::try_from("ws://127.0.0.1:8080/device/websocket?session=test_token").unwrap();
        assert_eq!(exp, url);
    }
}
