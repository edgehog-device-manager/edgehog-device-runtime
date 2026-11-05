// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Configures the Astarte Device SDK.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::Mergiable;

/// Connection type to use to connect with Astarte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AstarteLibrary {
    /// Uses the Astarte SDK via the Astarte MQTT protocol.
    #[default]
    #[serde(rename = "astarte-device-sdk")]
    AstarteDeviceSdk,
    /// Connects to the Astarte Message Hub.
    #[serde(rename = "astarte-message-hub")]
    AstarteMessageHub,
}

/// Configuration for the Astarte Device SDK
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceSdkConfig {
    /// The Astarte realm the device belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realm: Option<String>,
    /// A unique ID for the device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// The credentials secret used to authenticate with Astarte.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_secret: Option<String>,
    /// Token used to register the device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_token: Option<String>,
    /// Url to the Astarte pairing API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_url: Option<Url>,
    /// Ignores SSL error from the Astarte broker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_ssl: Option<bool>,
}

impl Mergiable for DeviceSdkConfig {
    fn merge(&mut self, value: Self) {
        let Self {
            realm,
            device_id,
            credentials_secret,
            pairing_token,
            pairing_url,
            ignore_ssl,
        } = value;

        self.realm = realm.or(self.realm.take());
        self.device_id = device_id.or(self.device_id.take());
        self.credentials_secret = credentials_secret.or(self.credentials_secret.take());
        self.pairing_token = pairing_token.or(self.pairing_token.take());
        self.pairing_url = pairing_url.or(self.pairing_url.take());
        self.ignore_ssl = ignore_ssl.or(self.ignore_ssl);
    }
}

/// Configuration to connect to the message-hub.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MessageHubConfig {
    /// The Endpoint of the Astarte Message Hub to connect to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Url>,
}

impl Mergiable for MessageHubConfig {
    fn merge(&mut self, value: Self) {
        let Self { endpoint } = value;

        self.endpoint = endpoint.or(self.endpoint.take());
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_test_utils::with_insta;
    use pretty_assertions::assert_eq;

    use super::*;

    pub(crate) fn mock_device_sdk_config() -> Option<DeviceSdkConfig> {
        Some(DeviceSdkConfig {
            realm: Some("test-realm".to_string()),
            device_id: Some("device_id".to_string()),
            credentials_secret: Some("credentials_secret".to_string()),
            pairing_token: Some("pairing_token".to_string()),
            pairing_url: Some(Url::parse("https://api.astarte.example.com/pairing").unwrap()),
            ignore_ssl: Some(false),
        })
    }

    pub(crate) fn mock_msg_hub_config() -> Option<MessageHubConfig> {
        Some(MessageHubConfig {
            endpoint: Some(Url::parse("http://[::1]:50051").unwrap()),
        })
    }

    #[test]
    fn device_sdk_config_roundtrip() {
        let exp = mock_device_sdk_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: DeviceSdkConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn device_sdk_config_empty_roundtrip() {
        let exp = DeviceSdkConfig::default();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: DeviceSdkConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn msg_hub_config_roundtrip() {
        let exp = mock_msg_hub_config().unwrap();

        let toml_str = toml::to_string_pretty(&exp).unwrap();
        let res: MessageHubConfig = toml::from_str(&toml_str).unwrap();

        assert_eq!(res, exp);

        with_insta!({
            insta::assert_snapshot!(toml_str);
        });
    }

    #[test]
    fn device_sdk_config_merge() {
        let mut base = DeviceSdkConfig {
            realm: Some("base-realm".to_string()),
            device_id: Some("base-id".to_string()),
            credentials_secret: None,
            pairing_token: Some("base-token".to_string()),
            pairing_url: None,
            ignore_ssl: Some(false),
        };

        let override_cfg = DeviceSdkConfig {
            realm: None,
            device_id: Some("new-id".to_string()),
            credentials_secret: Some("new-secret".to_string()),
            pairing_token: None,
            pairing_url: Some(Url::parse("https://pairing.example.com").unwrap()),
            ignore_ssl: Some(true),
        };

        let exp = DeviceSdkConfig {
            realm: Some("base-realm".to_string()),
            device_id: Some("new-id".to_string()),
            credentials_secret: Some("new-secret".to_string()),
            pairing_token: Some("base-token".to_string()),
            pairing_url: Some(Url::parse("https://pairing.example.com").unwrap()),
            ignore_ssl: Some(true),
        };

        base.merge(override_cfg);

        assert_eq!(base, exp);
    }

    #[test]
    fn msg_hub_config_merge() {
        let mut base = MessageHubConfig {
            endpoint: Some(Url::parse("http://[::1]:50051").unwrap()),
        };

        base.merge(MessageHubConfig { endpoint: None });
        let exp = MessageHubConfig {
            endpoint: Some(Url::parse("http://[::1]:50051").unwrap()),
        };
        assert_eq!(base, exp);

        base.merge(MessageHubConfig {
            endpoint: Some(Url::parse("http://[::1]:50052").unwrap()),
        });
        let exp = MessageHubConfig {
            endpoint: Some(Url::parse("http://[::1]:50052").unwrap()),
        };
        assert_eq!(base, exp);
    }
}
