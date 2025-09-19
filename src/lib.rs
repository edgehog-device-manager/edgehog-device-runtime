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

use std::path::PathBuf;

use serde::Deserialize;

pub use self::controller::Runtime;
use self::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
use self::telemetry::TelemetryInterfaceConfig;

cfg_if::cfg_if! {
    if #[cfg(test)] {
        // Used for the mocks
        pub use astarte_device_sdk_mock::Client;
    } else {
        pub use astarte_device_sdk::Client;
    }
}

mod commands;
#[cfg(feature = "containers")]
pub mod containers;
mod controller;
pub mod data;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod device;
pub mod error;
#[cfg(feature = "forwarder")]
mod forwarder;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod led_behavior;
#[cfg(all(feature = "zbus", target_os = "linux"))]
mod ota;
mod power_management;
pub mod repository;
#[cfg(feature = "systemd")]
pub mod systemd_wrapper;
pub mod telemetry;

#[derive(Deserialize, Debug, Clone)]
pub enum AstarteLibrary {
    #[serde(rename = "astarte-device-sdk")]
    AstarteDeviceSdk,
    #[cfg(feature = "message-hub")]
    #[serde(rename = "astarte-message-hub")]
    AstarteMessageHub,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DeviceManagerOptions {
    pub astarte_library: AstarteLibrary,
    pub astarte_device_sdk: Option<AstarteDeviceSdkConfigOptions>,
    #[cfg(feature = "message-hub")]
    pub astarte_message_hub: Option<data::astarte_message_hub_node::AstarteMessageHubOptions>,
    #[cfg(feature = "containers")]
    pub containers: containers::ContainersConfig,
    #[cfg(feature = "service")]
    pub service: Option<edgehog_service::config::Config>,
    pub interfaces_directory: PathBuf,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig<'static>>>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::Sequence;
    use tempdir::TempDir;
    use tokio::task::JoinSet;
    use tokio_util::sync::CancellationToken;
    use url::Url;

    use crate::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
    use crate::telemetry::tests::mock_initial_telemetry_client;
    use crate::Runtime;
    use crate::{AstarteLibrary, DeviceManagerOptions};

    #[cfg(feature = "forwarder")]
    fn mock_forwarder(
        publisher: &mut astarte_device_sdk_mock::MockDeviceClient<
            astarte_device_sdk::transport::mqtt::Mqtt<astarte_device_sdk::store::SqliteStore>,
        >,
        seq: &mut Sequence,
    ) {
        // define an expectation for the cloned MockPublisher due to the `init` method of the
        // Forwarder struct

        use astarte_device_sdk::store::SqliteStore;
        use astarte_device_sdk::transport::mqtt::Mqtt;
        use astarte_device_sdk_mock::MockDeviceClient;

        publisher
            .expect_clone()
            .once()
            .in_sequence(seq)
            .returning(move || {
                let mut publisher_clone = MockDeviceClient::<Mqtt<SqliteStore>>::new();

                publisher_clone
                    .expect_interface_props()
                    .withf(move |iface: &str| {
                        iface == "io.edgehog.devicemanager.ForwarderSessionState"
                    })
                    .returning(|_: &str| Ok(Vec::new()));

                publisher_clone
            });
    }

    #[tokio::test]
    async fn device_manager_new_success() {
        let tempdir = TempDir::new("device-manager-new").unwrap();

        let options = DeviceManagerOptions {
            astarte_library: AstarteLibrary::AstarteDeviceSdk,
            astarte_device_sdk: Some(AstarteDeviceSdkConfigOptions {
                realm: "".to_string(),
                device_id: Some("device_id".to_string()),
                credentials_secret: Some("credentials_secret".to_string()),
                pairing_url: Url::parse("http://[::]").unwrap(),
                pairing_token: None,
                ignore_ssl: false,
            }),
            #[cfg(feature = "message-hub")]
            astarte_message_hub: None,
            #[cfg(feature = "containers")]
            containers: crate::containers::ContainersConfig::default(),
            #[cfg(feature = "service")]
            service: None,
            interfaces_directory: PathBuf::new(),
            store_directory: tempdir.path().to_owned(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        #[cfg(feature = "zbus")]
        client
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        // telemetry
        client
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(mock_initial_telemetry_client);

        #[cfg(feature = "containers")]
        client
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(|| {
                let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
                let mut seq = Sequence::new();

                client
                    .expect_clone()
                    .once()
                    .in_sequence(&mut seq)
                    .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

                client
            });

        #[cfg(feature = "forwarder")]
        mock_forwarder(&mut client, &mut seq);

        let mut tasks = JoinSet::new();

        let _dm = Runtime::new(&mut tasks, options, client, CancellationToken::new())
            .await
            .unwrap();

        // Sleep to pass the execution to the telemetry task, this is somewhat of a bad test
        tokio::time::sleep(Duration::from_millis(100)).await;

        tasks.abort_all();

        while let Some(res) = tokio::time::timeout(Duration::from_secs(2), tasks.join_next())
            .await
            .unwrap()
        {
            match res {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    panic!("{err:#}");
                }
                Err(err) if err.is_cancelled() => {}
                Err(err) => {
                    panic!("{}", err);
                }
            }
        }
    }
}
