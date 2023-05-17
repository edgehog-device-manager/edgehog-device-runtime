/*
 * This file is part of Edgehog.
 *
 * Copyright 2023 SECO Mind Srl
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use astarte_device_sdk::types::AstarteType;
    use httpmock::prelude::*;
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;

    use crate::data::MockPublisher;
    use crate::error::DeviceManagerError;
    use crate::ota::ota_handle::{run_ota, Ota, PersistentState};
    use crate::ota::ota_handler::{OtaEvent, OtaHandler};
    use crate::ota::rauc::BundleInfo;
    use crate::ota::{MockSystemUpdate, OtaError};
    use crate::repository::MockStateRepository;

    impl OtaHandler {
        async fn mock_new(
            system_update: MockSystemUpdate,
            state_mock: MockStateRepository<PersistentState>,
        ) -> Result<Self, DeviceManagerError> {
            let (sender, receiver) = mpsc::channel(8);
            let ota = Ota::mock_new(system_update, state_mock);
            tokio::spawn(run_ota(ota, receiver));

            Ok(Self {
                sender,
                ota_cancellation: Arc::new(RwLock::new(None)),
            })
        }
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_not_compatible() {
        let bundle_info = "rauc-demo-x86";
        let system_info = "rauc-demo-arm";

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: bundle_info.to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok(system_info.to_string()));

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let uuid = Uuid::new_v4();
        let data = HashMap::from([
            ("url".to_string(), AstarteType::String(ota_url)),
            ("uuid".to_string(), AstarteType::String(uuid.to_string())),
            (
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            ),
        ]);

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let expected_message = format!(
            "bundle {} is not compatible with system {system_info}",
            bundle_info
        );
        let expected_message_cp = expected_message.clone();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Failure")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
                    && ota_event.statusCode.eq("InvalidBaseImage")
                    && ota_event.message.eq(&expected_message_cp)
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ota_event(&publisher, data).await;
        mock_ota_file_request.assert();

        assert!(result.is_err());
        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::InvalidBaseImage(error_message) = ota_error {
                assert_eq!(expected_message.to_string(), error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_install_completed_fail() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update
            .expect_receive_completed()
            .returning(|| Ok(-1));
        system_update.expect_install_bundle().returning(|_| Ok(()));
        system_update
            .expect_boot_slot()
            .returning(|| Ok("".to_owned()));
        system_update
            .expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let uuid = Uuid::new_v4();
        let data = HashMap::from([
            ("url".to_string(), AstarteType::String(ota_url)),
            ("uuid".to_string(), AstarteType::String(uuid.to_string())),
            (
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            ),
        ]);

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let expected_message = "Update failed with signal -1".to_string();
        let expected_message_cl = expected_message.clone();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Failure")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
                    && ota_event.statusCode.eq("InvalidBaseImage")
                    && ota_event.message.eq(&expected_message_cl)
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ota_event(&publisher, data).await;
        mock_ota_file_request.assert();

        assert!(result.is_err());

        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::InvalidBaseImage(error_message) = ota_error {
                assert_eq!(expected_message, error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn ota_event_fail_deployed() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_| Err(DeviceManagerError::FatalError("install fail".to_string())));

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Failure")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
                    && ota_event.statusCode.eq("InvalidBaseImage")
                    && ota_event.message.eq("Unable to install ota image")
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_err());
        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::InvalidBaseImage(error_message) = ota_error {
                assert_eq!("Unable to install ota image".to_string(), error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn ota_event_update_success() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_update_already_in_progress_same_uuid() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .delay(Duration::from_secs(3))
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let ota_handler_cp = ota_handler.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let mut ota_req_map = HashMap::new();
            ota_req_map.insert(
                "uuid".to_owned(),
                AstarteType::String(uuid.clone().to_string()),
            );
            ota_req_map.insert(
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            );

            let mut publisher = MockPublisher::new();

            publisher
                .expect_send_object()
                .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                    ota_event.status.eq("Downloading")
                        && ota_event.statusCode.eq("")
                        && ota_event.statusProgress == 0
                        && ota_event.requestUUID == uuid.to_string()
                        && ota_event.message.eq("")
                })
                .times(1)
                .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

            let result = ota_handler_cp.ota_event(&publisher, ota_req_map).await;
            assert!(result.is_err());

            if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
                assert_eq!(ota_error, OtaError::UpdateAlreadyInProgress)
            } else {
                panic!("Wrong DeviceManagerError type");
            }
        });

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());

        let result = handle.await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_update_already_in_progress_different_uuid() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .delay(Duration::from_secs(3))
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let ota_handler_cp = ota_handler.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;

            let uuid = Uuid::new_v4();
            let mut ota_req_map = HashMap::new();
            ota_req_map.insert(
                "uuid".to_owned(),
                AstarteType::String(uuid.clone().to_string()),
            );
            ota_req_map.insert(
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            );

            let mut publisher = MockPublisher::new();

            publisher
                .expect_send_object()
                .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                    ota_event.status.eq("Failure")
                        && ota_event.statusCode.eq("UpdateAlreadyInProgress")
                        && ota_event.statusProgress == 0
                        && ota_event.requestUUID == uuid.to_string()
                        && ota_event.message.eq("")
                })
                .times(1)
                .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

            let result = ota_handler_cp.ota_event(&publisher, ota_req_map).await;
            assert!(result.is_err());

            if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
                assert_eq!(ota_error, OtaError::UpdateAlreadyInProgress)
            } else {
                panic!("Wrong DeviceManagerError type");
            }
        });

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());

        let result = handle.await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_cancelled() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .delay(Duration::from_secs(5))
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();

        let ota_handler_cp = ota_handler.clone();
        let cancel_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut ota_req_map = HashMap::new();
            ota_req_map.insert(
                "uuid".to_owned(),
                AstarteType::String(uuid.clone().to_string()),
            );
            ota_req_map.insert(
                "operation".to_string(),
                AstarteType::String("Cancel".to_string()),
            );

            let mut publisher = MockPublisher::new();

            publisher
                .expect_send_object()
                .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                    ota_event.status.eq("Failure")
                        && ota_event.statusCode.eq("Cancelled")
                        && ota_event.statusProgress == 0
                        && ota_event.requestUUID == uuid.to_string()
                })
                .times(1)
                .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

            let result = ota_handler_cp.ota_event(&publisher, ota_req_map).await;
            assert!(result.is_ok());
        });

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();
        assert!(result.is_ok());

        let result = cancel_handle.await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_not_cancelled() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let ota_handler_cp = ota_handler.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut ota_req_map = HashMap::new();
            ota_req_map.insert(
                "uuid".to_owned(),
                AstarteType::String(uuid.clone().to_string()),
            );
            ota_req_map.insert(
                "operation".to_string(),
                AstarteType::String("Cancel".to_string()),
            );

            let mut publisher = MockPublisher::new();

            publisher
                .expect_send_object()
                .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                    ota_event.status.eq("Failure")
                        && ota_event.statusCode.eq("InternalError")
                        && ota_event.statusProgress == 0
                        && ota_event.requestUUID == uuid.to_string()
                        && ota_event.message.eq("Unable to cancel OTA request")
                })
                .times(1)
                .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

            let result = ota_handler_cp.ota_event(&publisher, ota_req_map).await;
            assert!(result.is_ok());
        });

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());

        let result = handle.await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_not_cancelled_different_uuid() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_install_bundle()
            .returning(|_: &str| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .delay(Duration::from_secs(5))
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url));
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        ota_req_map.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let ota_handler_cp = ota_handler.clone();

        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let mut ota_req_map = HashMap::new();
            let uuid_diff = Uuid::new_v4();
            ota_req_map.insert(
                "uuid".to_owned(),
                AstarteType::String(uuid_diff.clone().to_string()),
            );
            ota_req_map.insert(
                "operation".to_string(),
                AstarteType::String("Cancel".to_string()),
            );

            let mut publisher = MockPublisher::new();

            publisher
                .expect_send_object()
                .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                    ota_event.status.eq("Failure")
                        && ota_event.statusCode.eq("InternalError")
                        && ota_event.statusProgress == 0
                        && ota_event.requestUUID == uuid_diff.to_string()
                        && ota_event
                            .message
                            .eq("Unable to cancel OTA request, they have different identifier")
                })
                .times(1)
                .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

            let result = ota_handler_cp.ota_event(&publisher, ota_req_map).await;
            assert!(result.is_ok());
        });

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());

        let result = handle.await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ensure_pending_ota_response_fail() {
        let uuid = Uuid::new_v4();
        let slot = "A";

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update
            .expect_boot_slot()
            .returning(|| Ok("A".to_owned()));

        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Failure")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
                    && ota_event.statusCode.eq("SystemRollback")
                    && ota_event.message.eq("Unable to switch slot")
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ensure_pending_ota_response(&publisher).await;
        assert!(result.is_err());

        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::SystemRollback(error_message) = ota_error {
                assert_eq!("Unable to switch slot", error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn ensure_pending_ota_response_ota_success() {
        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut system_update = MockSystemUpdate::new();
        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let mut publisher = MockPublisher::new();
        let mut seq = mockall::Sequence::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
            .in_sequence(&mut seq);

        let ota_handler = OtaHandler::mock_new(system_update, state_mock)
            .await
            .unwrap();
        let result = ota_handler.ensure_pending_ota_response(&publisher).await;

        assert!(result.is_ok());
    }
}
