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
pub(crate) mod tls;

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
    pub interfaces_directory: PathBuf,
    pub store_directory: PathBuf,
    pub download_directory: PathBuf,
    pub telemetry_config: Option<Vec<TelemetryInterfaceConfig<'static>>>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use mockall::Sequence;
    use tempdir::TempDir;
    use tokio::task::JoinSet;
    use url::Url;

    use crate::data::astarte_device_sdk_lib::AstarteDeviceSdkConfigOptions;
    use crate::data::tests::create_tmp_store;
    use crate::data::tests::MockPubSub;
    use crate::telemetry::tests::mock_initial_telemetry_client;
    use crate::Runtime;
    use crate::{AstarteLibrary, DeviceManagerOptions};

    #[cfg(feature = "forwarder")]
    fn mock_forwarder(publisher: &mut MockPubSub, seq: &mut Sequence) {
        // define an expectation for the cloned MockPublisher due to the `init` method of the
        // Forwarder struct
        publisher
            .expect_clone()
            .once()
            .in_sequence(seq)
            .returning(move || {
                let mut publisher_clone = MockPubSub::new();

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
    #[should_panic]
    async fn device_new_sdk_panic_fail() {
        let (store, store_dir) = create_tmp_store().await;

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
            interfaces_directory: PathBuf::new(),
            store_directory: store_dir.path().to_owned(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let mut tasks = JoinSet::new();

        let client = options
            .astarte_device_sdk
            .as_ref()
            .unwrap()
            .connect(
                &mut tasks,
                store,
                &options.store_directory,
                &options.interfaces_directory,
            )
            .await
            .unwrap();

        let dm = Runtime::new(&mut tasks, options, client).await;

        assert!(dm.is_ok());
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
            interfaces_directory: PathBuf::new(),
            store_directory: tempdir.path().to_owned(),
            download_directory: PathBuf::new(),
            telemetry_config: Some(vec![]),
        };

        let mut pub_sub = MockPubSub::new();
        let mut seq = Sequence::new();

        #[cfg(feature = "zbus")]
        pub_sub
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockPubSub::new);

        // telemetry
        pub_sub
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(mock_initial_telemetry_client);

        #[cfg(feature = "containers")]
        pub_sub
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(|| {
                let mut client = MockPubSub::new();
                let mut seq = Sequence::new();

                client
                    .expect_clone()
                    .once()
                    .in_sequence(&mut seq)
                    .returning(MockPubSub::new);

                client
            });

        #[cfg(feature = "forwarder")]
        mock_forwarder(&mut pub_sub, &mut seq);

        let mut tasks = JoinSet::new();

        let _dm = Runtime::new(&mut tasks, options, pub_sub).await.unwrap();

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
