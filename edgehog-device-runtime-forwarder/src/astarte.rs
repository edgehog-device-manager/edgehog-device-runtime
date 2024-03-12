// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Implement the interaction with the [Astarte rust SDK](astarte_device_sdk).
//!
//! Module responsible for handling a connection between a Device and Astarte.

use std::hash::Hash;
use std::{collections::HashMap, num::TryFromIntError};

use astarte_device_sdk::{types::AstarteType, AstarteAggregate, Error as SdkError};
use thiserror::Error;
use tracing::instrument;
use url::{Host, ParseError, Url};

/// Astarte errors.
#[non_exhaustive]
#[derive(displaydoc::Display, Error, Debug)]
pub enum AstarteError {
    /// Error occurring when different fields from those of the mapping are received.
    Sdk(#[from] SdkError),

    /// Missing url information, `{0}`.
    MissingUrlInfo(&'static str),

    /// Error while parsing an url, `{0}`.
    ParseUrl(#[from] ParseError),

    /// Received a malformed port number, `{0}`.
    ParsePort(#[from] TryFromIntError),

    /// Wrong path on astarte interface, {0}.
    WrongPath(String),

    /// Received Individual rather than Aggregation Astarte data type.
    WrongData,
}

/// Struct representing the fields of an aggregated object the Astarte server can send to the device.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SessionInfo {
    /// Hostname or IP address.
    pub host: Host,
    /// Port number.
    pub port: u16,
    /// Session token.
    pub session_token: String,
}

impl Hash for SessionInfo {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.host.hash(state);
        self.port.hash(state);
        self.session_token.hash(state);
    }
}

impl AstarteAggregate for SessionInfo {
    fn astarte_aggregate(self) -> Result<HashMap<String, AstarteType>, SdkError> {
        let mut hm = HashMap::new();
        hm.insert("host".to_string(), self.host.to_string().into());
        hm.insert("port".to_string(), AstarteType::Integer(self.port.into()));
        hm.insert("session_token".to_string(), self.session_token.into());
        Ok(hm)
    }
}

impl TryFrom<&SessionInfo> for Url {
    type Error = AstarteError;

    fn try_from(value: &SessionInfo) -> Result<Self, Self::Error> {
        if value.session_token.is_empty() {
            return Err(AstarteError::MissingUrlInfo("missing session token"));
        }

        Url::parse_with_params(
            &format!("ws://{}:{}/device/websocket", value.host, value.port),
            &[("session", &value.session_token)],
        )
        .map_err(AstarteError::ParseUrl)
    }
}

/// Parse an `HashMap` containing pairs (Endpoint, [`AstarteType`]) into an URL.
#[instrument(skip_all)]
pub fn retrieve_session_info(
    mut map: HashMap<String, AstarteType>,
) -> Result<SessionInfo, AstarteError> {
    let host = map
        .remove("host")
        .ok_or_else(|| AstarteError::MissingUrlInfo("Missing host (IP or domain name)"))
        .and_then(|t| t.try_into().map_err(|e| SdkError::Types(e).into()))
        .and_then(|host: String| Host::parse(&host).map_err(AstarteError::from))?;

    let port: u16 = map
        .remove("port")
        .ok_or_else(|| AstarteError::MissingUrlInfo("Missing port value"))
        .and_then(|t| t.try_into().map_err(|e| SdkError::Types(e).into()))
        .and_then(|port: i32| port.try_into().map_err(AstarteError::from))?;

    let session_token: String = map
        .remove("session_token")
        .ok_or_else(|| AstarteError::MissingUrlInfo("Missing session_token"))?
        .try_into()
        .map_err(SdkError::Types)?;

    Ok(SessionInfo {
        host,
        port,
        session_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn create_sinfo(token: &str) -> SessionInfo {
        SessionInfo {
            host: Host::Ipv4(Ipv4Addr::LOCALHOST),
            port: 8080,
            session_token: token.to_string(),
        }
    }

    fn create_astarte_hashmap(
        host: &str,
        port: i32,
        session_token: &str,
    ) -> HashMap<String, AstarteType> {
        let mut hm = HashMap::new();

        if !host.is_empty() {
            hm.insert("host".to_string(), AstarteType::String(host.to_string()));
        }
        if port.is_positive() {
            hm.insert("port".to_string(), AstarteType::Integer(port));
        }

        if !session_token.is_empty() {
            hm.insert(
                "session_token".to_string(),
                AstarteType::String(session_token.to_string()),
            );
        }

        hm
    }

    #[test]
    fn test_astarte_aggregate() {
        let sinfo = create_sinfo("test_token");

        let expected = [
            ("host", AstarteType::String("127.0.0.1".to_string())),
            ("port", AstarteType::Integer(8080)),
            (
                "session_token",
                AstarteType::String("test_token".to_string()),
            ),
        ];

        let res = sinfo.astarte_aggregate();

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
            create_astarte_hashmap("", 8080, "test_token"),
            create_astarte_hashmap("127.0.0.1", 0, "test_token"),
            create_astarte_hashmap("127.0.0.1", 8080, ""),
        ];

        for hm in err_cases {
            assert!(retrieve_session_info(hm).is_err());
        }

        let hm = create_astarte_hashmap("127.0.0.1", 8080, "test_token");
        let sinfo = retrieve_session_info(hm).unwrap();

        assert_eq!(sinfo.host, Host::<&str>::Ipv4(Ipv4Addr::LOCALHOST));
        assert_eq!(sinfo.port, 8080);
        assert_eq!(sinfo.session_token, "test_token".to_string());
    }
}
