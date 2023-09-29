// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Implement the interaction with the [Astarte rust SDK](astarte_device_sdk).
//!
//! Module responsible for handling a connection between a Device and Astarte.

use std::{collections::HashMap, num::TryFromIntError};

use astarte_device_sdk::{types::AstarteType, AstarteAggregate, Error as SdkError};
use displaydoc::Display;
use thiserror::Error;
use tracing::instrument;
use url::{Host, ParseError, Url};

/// Astarte errors.
#[non_exhaustive]
#[derive(Display, Error, Debug)]
pub enum AstarteError {
    /// Error occurring when different fields from those of the mapping are received.
    Sdk(#[from] SdkError),

    /// Missing url information, `{0}`.
    MissingUrlInfo(&'static str),

    /// Error while parsing an url, `{0}`.
    ParseUrl(#[from] ParseError),

    /// Received a malformed port number, `{0}`.
    ParsePort(#[from] TryFromIntError),
}

/// Struct representing the fields of an aggregated object the Astarte server can send to the device.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    /// Hostname or IP address.
    pub host: Host,
    /// Port number.
    pub port: u16,
    session_token: String,
}

impl AstarteAggregate for ConnectionInfo {
    fn astarte_aggregate(self) -> Result<HashMap<String, AstarteType>, SdkError> {
        let mut hm = HashMap::new();
        hm.insert("host".to_string(), self.host.to_string().into());
        hm.insert("port".to_string(), AstarteType::Integer(self.port.into()));
        hm.insert("session_token".to_string(), self.session_token.into());
        Ok(hm)
    }
}

impl TryFrom<&ConnectionInfo> for Url {
    type Error = AstarteError;

    fn try_from(value: &ConnectionInfo) -> Result<Self, Self::Error> {
        Url::parse_with_params(
            &format!("ws://{}:{}/path", value.host, value.port),
            &[("session_token", &value.session_token)],
        )
        .map_err(AstarteError::ParseUrl)
    }
}

/// Parse an `HashMap` containing pairs (Endpoint, [`AstarteType`]) into an URL.
#[instrument(skip_all)]
pub fn retrieve_connection_info(
    mut map: HashMap<String, AstarteType>,
) -> Result<ConnectionInfo, AstarteError> {
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

    Ok(ConnectionInfo {
        host,
        port,
        session_token,
    })
}
