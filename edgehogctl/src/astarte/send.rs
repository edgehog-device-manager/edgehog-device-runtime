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

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use color_eyre::{
    Section, SectionExt,
    eyre::{WrapErr, eyre},
    owo_colors::OwoColorize,
};
use eyre::OptionExt;
use reqwest::header;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};
use url::Url;

use crate::client::{ApiData, gen_jwt_token, http_client};

/// Subset of the Edgehog device runtime configuration file needed to reach Astarte.
#[derive(Debug, Deserialize, Clone, Default)]
struct Config {
    astarte_device_sdk: Option<DeviceSdkArgs>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeviceSdkArgs {
    /// The Astarte realm the device belongs to.
    realm: Option<String>,
    /// A unique ID for the device.
    device_id: Option<String>,
    /// URL to the Astarte pairing API.
    pairing_url: Option<Url>,
}

#[derive(Debug, Clone, clap::Args)]
pub(crate) struct SendData {
    /// Edgehog device config
    #[arg(long)]
    config: PathBuf,
    /// Prints the requests as "curl" commands.
    #[arg(long, default_value_t = false)]
    curl: bool,
    /// Astarte JWT token (if omitted, generated via `astartectl utils gen-jwt all-realm-apis`).
    #[arg(long)]
    token: Option<String>,
    /// Path to a json file containing the data to send.
    data: PathBuf,
}

impl SendData {
    #[instrument(skip_all)]
    pub(crate) async fn run(self) -> eyre::Result<()> {
        let config = tokio::fs::read_to_string(&self.config)
            .await
            .wrap_err_with(|| format!("couldn't read config file {}", self.config.display()))?;
        let config: Config = toml::from_str(&config)
            .wrap_err_with(|| format!("couldn't parse config file {}", self.config.display()))?;

        let client = ApiClient::new(config)?;

        if self.curl {
            client.print_curl(&self.data).await?;
        } else {
            let token = match self.token {
                Some(token) => token,
                None => gen_jwt_token().await?,
            };
            client.read(&self.data, &token).await?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct ApiClient {
    client: reqwest::Client,
    url: Url,
}

impl ApiClient {
    fn new(config: Config) -> eyre::Result<Self> {
        let url = Self::build_base_url(config)?;
        let client = http_client()?;

        Ok(Self { client, url })
    }

    fn build_base_url(config: Config) -> eyre::Result<Url> {
        let config = config
            .astarte_device_sdk
            .ok_or_eyre("missing astarte configuration")?;

        let realm = config.realm.ok_or_eyre("missing realm")?;
        let device_id = config.device_id.ok_or_eyre("missing device id")?;

        let mut url = config.pairing_url.ok_or_eyre("missing pairing url")?;
        url.path_segments_mut()
            .map_err(|()| eyre!("invalid url"))?
            // Remove trailing empty segment (if any) and `pairing`
            .pop_if_empty()
            .pop()
            .extend([
                "appengine",
                "v1",
                &realm,
                "devices",
                &device_id,
                "interfaces",
            ]);

        Ok(url)
    }

    async fn send<T>(&self, token: &str, interface: &str, path: &str, data: T) -> eyre::Result<()>
    where
        T: Serialize + Debug,
    {
        let url = self.interface_url(interface, path)?;

        let res = self
            .client
            .post(url.clone())
            .bearer_auth(token)
            .header(header::ACCEPT, "application/json")
            .json(&ApiData { data: &data })
            .send()
            .await?;

        let status = res.status();

        let text: serde_json::Value = res.json().await.wrap_err("couldn't get response body")?;

        if !status.is_success() {
            let err = eyre!("HTTP status {status} for ({interface}{path})")
                .with_section(move || format!("{url}").header("Url:"))
                .with_section(move || format!("{:#?}", data.dimmed()).header("Request:"))
                .with_section(move || format!("{:#}", text.red()).header("Response:"));

            return Err(err);
        }

        info!("response: {text}");

        Ok(())
    }

    fn interface_url(&self, interface: &str, path: &str) -> eyre::Result<Url> {
        let mut url = self.url.clone();
        url.path_segments_mut()
            .map_err(|()| eyre!("couldn't get the url path {}", self.url))?
            .push(interface)
            .extend(
                path.trim_start_matches('/')
                    .split('/')
                    .filter(|segment| !segment.is_empty()),
            );
        Ok(url)
    }

    async fn read(&self, path: &Path, token: &str) -> eyre::Result<()> {
        let content = tokio::fs::read_to_string(path).await?;

        let values: EventsFile = serde_json::from_str(&content)?;

        for v in values.events {
            self.send(token, &v.interface, &v.path, v.data).await?;
        }

        Ok(())
    }

    async fn print_curl(&self, path: &Path) -> eyre::Result<()> {
        let content = tokio::fs::read_to_string(path).await?;

        let values: EventsFile = serde_json::from_str(&content)?;

        println!("#!/usr/bin/env bash");
        println!();
        println!("set -exEuo pipefail");
        println!();

        for v in values.events {
            self.print_send(v)?;
        }

        Ok(())
    }

    fn print_send(&self, data: InterfaceData) -> eyre::Result<()> {
        println!(
            r#"curl -v -X POST -H "Accept: application/json" -H "Content-Type: application/json" \"#
        );
        println!(r#"    -H "Authorization: Bearer $TOKEN" \"#);
        println!(
            r#"    --data '{}' \"#,
            serde_json::to_string(&ApiData { data: &data.data })?
        );

        println!("    '{}'", self.interface_url(&data.interface, &data.path)?);

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InterfaceData {
    interface: String,
    path: String,
    data: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct EventsFile {
    events: Vec<InterfaceData>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(pairing_url: &str) -> Config {
        Config {
            astarte_device_sdk: Some(DeviceSdkArgs {
                realm: Some("test".to_string()),
                device_id: Some("device-123".to_string()),
                pairing_url: Some(Url::parse(pairing_url).unwrap()),
            }),
        }
    }

    #[test]
    fn build_base_url_strips_pairing_segment() {
        let url = ApiClient::build_base_url(sample_config("http://api.astarte.localhost/pairing"))
            .unwrap();
        assert_eq!(
            url.as_str(),
            "http://api.astarte.localhost/appengine/v1/test/devices/device-123/interfaces"
        );

        let url_trailing =
            ApiClient::build_base_url(sample_config("http://api.astarte.localhost/pairing/"))
                .unwrap();
        assert_eq!(
            url_trailing.as_str(),
            "http://api.astarte.localhost/appengine/v1/test/devices/device-123/interfaces"
        );
    }

    #[test]
    fn build_base_url_missing_fields_returns_err() {
        assert!(ApiClient::build_base_url(Config::default()).is_err());

        let missing_realm = Config {
            astarte_device_sdk: Some(DeviceSdkArgs {
                realm: None,
                device_id: Some("device-123".to_string()),
                pairing_url: Some(Url::parse("http://api.astarte.localhost/pairing").unwrap()),
            }),
        };
        assert!(ApiClient::build_base_url(missing_realm).is_err());
    }

    #[test]
    fn interface_url_handles_single_and_multi_segment_paths() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let client = ApiClient::new(sample_config("http://api.astarte.localhost/pairing")).unwrap();

        let single = client
            .interface_url("io.edgehog.devicemanager.apps.CreateImageRequest", "/image")
            .unwrap();
        assert_eq!(
            single.as_str(),
            "http://api.astarte.localhost/appengine/v1/test/devices/device-123/interfaces/io.edgehog.devicemanager.apps.CreateImageRequest/image"
        );

        let nested = client
            .interface_url("io.edgehog.devicemanager.Example", "/foo/bar")
            .unwrap();
        assert_eq!(
            nested.as_str(),
            "http://api.astarte.localhost/appengine/v1/test/devices/device-123/interfaces/io.edgehog.devicemanager.Example/foo/bar"
        );
    }

    #[test]
    fn deserialize_events_file() {
        let raw = r#"{
            "events": [
                {
                    "interface": "io.edgehog.devicemanager.apps.CreateImageRequest",
                    "path": "/image",
                    "data": { "id": "abc" }
                }
            ]
        }"#;

        let parsed: EventsFile = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(
            parsed.events[0].interface,
            "io.edgehog.devicemanager.apps.CreateImageRequest"
        );
        assert_eq!(parsed.events[0].path, "/image");
    }
}
