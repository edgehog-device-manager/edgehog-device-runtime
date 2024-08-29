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

use std::collections::HashMap;
use std::sync::Arc;

use crate::data::tests::MockPubSub;
use astarte_device_sdk::types::AstarteType;
use futures::StreamExt;
use httpmock::prelude::*;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::DeviceManagerError;
use crate::ota::ota_handle::{run_ota, Ota, OtaRequest, OtaStatus, PersistentState};
use crate::ota::ota_handler::{OtaEvent, OtaHandler};
use crate::ota::rauc::BundleInfo;
use crate::ota::{DeployStatus, MockSystemUpdate, OtaError, ProgressStream};
use crate::repository::MockStateRepository;

pub(crate) fn deploy_status_stream<I>(iter: I) -> Result<ProgressStream, DeviceManagerError>
where
    I: IntoIterator<Item = DeployStatus>,
{
    Ok(futures::stream::iter(iter.into_iter().map(Ok).collect::<Vec<_>>()).boxed())
}

impl OtaHandler {
    fn mock_new(
        system_update: MockSystemUpdate,
        state_repository: MockStateRepository<PersistentState>,
    ) -> Self {
        let ota = Ota::mock_new(system_update, state_repository);

        Self::mock_new_with_ota(ota)
    }

    fn mock_new_with_path(
        system_update: MockSystemUpdate,
        state_repository: MockStateRepository<PersistentState>,
        prefix: &str,
    ) -> (Self, tempdir::TempDir) {
        let (ota, dir) = Ota::mock_new_with_path(system_update, state_repository, prefix);

        let handler = Self::mock_new_with_ota(ota);

        (handler, dir)
    }

    fn mock_new_with_ota(ota: Ota<MockSystemUpdate, MockStateRepository<PersistentState>>) -> Self {
        let (sender, receiver) = mpsc::channel(8);

        tokio::spawn(run_ota(ota, receiver));

        Self {
            sender,
            ota_cancellation: Arc::new(RwLock::new(None)),
        }
    }
}

#[tokio::test]
async fn handle_ota_event_bundle_not_compatible() {
    let bundle_info = "rauc-demo-x86";
    let system_info = "rauc-demo-arm";

    let mut state_mock = MockStateRepository::<PersistentState>::new();
    state_mock.expect_write().returning(|_| Ok(()));
    state_mock.expect_clear().returning(|| Ok(()));
    state_mock.expect_exists().returning(|| true);

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

    let server = MockServer::start_async().await;
    let ota_url = server.url("/ota.bin");
    let mock_ota_file_request = server
        .mock_async(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        })
        .await;

    let uuid = Uuid::new_v4();
    let data = HashMap::from([
        ("url".to_string(), AstarteType::String(ota_url)),
        ("uuid".to_string(), AstarteType::String(uuid.to_string())),
        (
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        ),
    ]);

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();
    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Acknowledged")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 100
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let expected_message = format!(
        "bundle {} is not compatible with system {system_info}",
        bundle_info
    );
    let expected_message_cp = expected_message.clone();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event.statusCode.eq("InvalidBaseImage")
                && ota_event.message.eq(&expected_message_cp)
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "bundle_not_compatible");
    let result = ota_handler.ota_event(&pub_sub, data).await;
    mock_ota_file_request.assert_async().await;

    assert!(result.is_err());
    if let DeviceManagerError::Ota(ota_error) = result.err().unwrap() {
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
    state_mock.expect_clear().returning(|| Ok(()));
    state_mock.expect_exists().returning(|| true);

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
        .returning(|| deploy_status_stream([DeployStatus::Completed { signal: -1 }]));
    system_update.expect_install_bundle().returning(|_| Ok(()));
    system_update
        .expect_boot_slot()
        .returning(|| Ok("".to_owned()));
    system_update
        .expect_last_error()
        .returning(|| Ok("Unable to deploy image".to_string()));

    let binary_content = b"\x80\x02\x03";
    let binary_size = binary_content.len();

    let server = MockServer::start_async().await;
    let ota_url = server.url("/ota.bin");
    let mock_ota_file_request = server
        .mock_async(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        })
        .await;

    let uuid = Uuid::new_v4();
    let data = HashMap::from([
        ("url".to_string(), AstarteType::String(ota_url)),
        ("uuid".to_string(), AstarteType::String(uuid.to_string())),
        (
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        ),
    ]);

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();
    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Acknowledged")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 100
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Deploying")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let expected_message = "Update failed with signal -1".to_string();
    let expected_message_cl = expected_message.clone();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event.statusCode.eq("InvalidBaseImage")
                && ota_event.message.eq(&expected_message_cl)
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "install_completed_fail");
    let result = ota_handler.ota_event(&pub_sub, data).await;
    mock_ota_file_request.assert_async().await;

    assert!(result.is_err());

    if let DeviceManagerError::Ota(ota_error) = result.err().unwrap() {
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
        .returning(|_| Err(DeviceManagerError::Fatal("install fail".to_string())));

    let binary_content = b"\x80\x02\x03";
    let binary_size = binary_content.len();

    let server = MockServer::start_async().await;
    let ota_url = server.url("/ota.bin");
    let mock_ota_file_request = server
        .mock_async(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        })
        .await;

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

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Acknowledged")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Downloading")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 100
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Deploying")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event.statusCode.eq("InvalidBaseImage")
                && ota_event.message.eq("Unable to install ota image")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "fail_deployed");
    let result = ota_handler.ota_event(&pub_sub, ota_req_map).await;
    mock_ota_file_request.assert_async().await;

    assert!(result.is_err());
    if let DeviceManagerError::Ota(ota_error) = result.err().unwrap() {
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
    system_update
        .expect_receive_completed()
        .once()
        .returning(|| deploy_status_stream([DeployStatus::Completed { signal: 0 }]));
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

    let server = MockServer::start_async().await;
    let ota_url = server.url("/ota.bin");
    let mock_ota_file_request = server
        .mock_async(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        })
        .await;

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

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "update_success");
    let result = ota_handler.ota_event(&pub_sub, ota_req_map).await;
    mock_ota_file_request.assert_async().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn ota_event_update_already_in_progress_same_uuid() {
    let uuid = Uuid::new_v4();

    let state_mock = MockStateRepository::<PersistentState>::new();

    let system_update = MockSystemUpdate::new();

    let ota_url = "http://localhost".to_string();

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url.clone()));
    ota_req_map.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Update".to_string()),
    );

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Acknowledged")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let ota = Ota::mock_new(system_update, state_mock);
    // Fake another update is happening state != idle
    *ota.ota_status.write().await = OtaStatus::Acknowledged(OtaRequest { uuid, url: ota_url });

    let ota_handler = OtaHandler::mock_new_with_ota(ota);

    let err = ota_handler
        .ota_event(&pub_sub, ota_req_map)
        .await
        .expect_err("expected already in progress error");

    assert!(
        matches!(
            err,
            DeviceManagerError::Ota(OtaError::UpdateAlreadyInProgress)
        ),
        "expected error for update already in progress but got: {}",
        err
    );
}

#[tokio::test]
async fn ota_event_update_already_in_progress_different_uuid() {
    let uuid_1 = Uuid::new_v4();
    let uuid_2 = Uuid::new_v4();

    let state_mock = MockStateRepository::<PersistentState>::new();

    let system_update = MockSystemUpdate::new();

    let ota_url = "http://localhost".to_string();

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert("url".to_owned(), AstarteType::String(ota_url.clone()));
    ota_req_map.insert("uuid".to_owned(), AstarteType::String(uuid_1.to_string()));
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Update".to_string()),
    );

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("UpdateAlreadyInProgress")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid_1.to_string()
                && ota_event.message.eq("")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let ota = Ota::mock_new(system_update, state_mock);
    // Fake another update is happening state != idle
    *ota.ota_status.write().await = OtaStatus::Acknowledged(OtaRequest {
        uuid: uuid_2,
        url: ota_url,
    });

    let ota_handler = OtaHandler::mock_new_with_ota(ota);

    let err = ota_handler
        .ota_event(&pub_sub, ota_req_map)
        .await
        .expect_err("expected already in progress error");

    assert!(
        matches!(
            err,
            DeviceManagerError::Ota(OtaError::UpdateAlreadyInProgress)
        ),
        "expected error for update already in progress but got: {}",
        err
    );
}

#[tokio::test]
async fn ota_event_canceled() {
    let uuid = Uuid::new_v4();
    let cancel_token = CancellationToken::new();

    let state_repository = MockStateRepository::<PersistentState>::new();
    let system_update = MockSystemUpdate::new();

    let ota = Ota::mock_new(system_update, state_repository);
    *ota.ota_status.write().await = OtaStatus::Acknowledged(OtaRequest {
        uuid,
        url: "".to_string(),
    });

    let ota_handler = OtaHandler::mock_new_with_ota(ota);
    *ota_handler.ota_cancellation.write().await = Some(cancel_token.clone());

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Cancel".to_string()),
    );

    let mut pub_sub = MockPubSub::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("Canceled")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let res = ota_handler.ota_event(&pub_sub, ota_req_map).await;

    assert!(res.is_ok(), "ota cancel failed with: {}", res.unwrap_err());

    assert!(cancel_token.is_cancelled());
}

#[tokio::test]
async fn ota_event_success_after_canceled_event() {
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
    system_update
        .expect_receive_completed()
        .returning(|| deploy_status_stream([DeployStatus::Completed { signal: 0 }]));
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

    let server = MockServer::start_async().await;
    let file_req = server
        .mock_async(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("Content-Length", binary_size.to_string())
                .body(binary_content);
        })
        .await;
    let ota_url = server.url("/ota.bin");

    let mut ota_update = HashMap::new();
    ota_update.insert("url".to_owned(), AstarteType::String(ota_url.clone()));
    ota_update.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_update.insert(
        "operation".to_string(),
        AstarteType::String("Update".to_string()),
    );

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "after_cancelled");

    // Start the update but handle the flow, so we can cancel it.
    let mut rx_update = ota_handler
        .start_ota_update(ota_update)
        .await
        .expect("failed to start ota");

    let ack = rx_update.recv().await.expect("failed to receive ack");
    assert_eq!(
        ack,
        OtaStatus::Acknowledged(OtaRequest {
            uuid,
            url: ota_url.clone()
        })
    );

    // We send the cancel event in another thread and wait for the response
    let mut ota_cancel = HashMap::new();
    ota_cancel.insert("uuid".to_owned(), AstarteType::String(uuid.to_string()));
    ota_cancel.insert(
        "operation".to_string(),
        AstarteType::String("Cancel".to_string()),
    );

    let mut pub_sub = MockPubSub::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("Canceled")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let ota_handler_cl = ota_handler.clone();
    let cancel_handle =
        tokio::spawn(async move { ota_handler_cl.ota_event(&pub_sub, ota_cancel).await });

    // Receive the next OTA event
    let status = rx_update.recv().await.expect("ota should be downloading");
    assert_eq!(
        status,
        OtaStatus::Downloading(
            OtaRequest {
                uuid,
                url: ota_url.clone()
            },
            0
        )
    );
    let status = rx_update.recv().await;
    assert!(status.is_none(), "ota should be cancelled");

    // Join with the cancel
    cancel_handle
        .await
        .expect("failed to join")
        .expect("cancel ota successfully");

    let mut ota_update = HashMap::new();
    ota_update.insert("url".to_owned(), AstarteType::String(ota_url));
    ota_update.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_update.insert(
        "operation".to_string(),
        AstarteType::String("Update".to_string()),
    );

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    pub_sub
        .expect_send_object()
        .withf(
            move |interface_name: &str, path: &str, ota_event: &OtaEvent| {
                interface_name.eq("io.edgehog.devicemanager.OTAEvent")
                    && path.eq("/event")
                    && ota_event.status.eq("Success")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            },
        )
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let result = ota_handler.ota_event(&pub_sub, ota_update).await;
    assert!(result.is_ok(), "update should succeed");

    // One for the cancelled and one for the successful
    let hits = file_req.hits_async().await;
    assert!(hits == 1 || hits == 2);
}

/// Try to cancel an OTA without a cancel token (OTA finished, same uuid)
#[tokio::test]
async fn ota_event_not_canceled() {
    let uuid = Uuid::new_v4();

    let state_mock = MockStateRepository::<PersistentState>::new();
    let system_update = MockSystemUpdate::new();

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Cancel".to_string()),
    );

    let mut pub_sub = MockPubSub::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("InternalError")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event.message.eq("Unable to cancel OTA request")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let (ota, _dir) = Ota::mock_new_with_path(system_update, state_mock, "not_cancelled");
    *ota.ota_status.write().await = OtaStatus::Success(OtaRequest {
        uuid,
        url: "".to_string(),
    });
    let ota_handler = OtaHandler::mock_new_with_ota(ota);

    let result = ota_handler.ota_event(&pub_sub, ota_req_map).await;

    assert!(
        result.is_ok(),
        "expected ota event to be Ok, but got: {}",
        result.unwrap_err()
    );
}

/// Try to cancel an OTA without a cancel token (no OTA started)
#[tokio::test]
async fn ota_event_not_canceled_empty() {
    let uuid = Uuid::new_v4();

    let state_mock = MockStateRepository::<PersistentState>::new();
    let system_update = MockSystemUpdate::new();

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Cancel".to_string()),
    );

    let mut pub_sub = MockPubSub::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("InternalError")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event
                    .message
                    .eq("Unable to cancel OTA request, internal request is empty")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let (ota_handler, _dir) =
        OtaHandler::mock_new_with_path(system_update, state_mock, "not_cancelled_empty");

    let result = ota_handler.ota_event(&pub_sub, ota_req_map).await;

    assert!(
        result.is_ok(),
        "expected ota event to be Ok, but got: {}",
        result.unwrap_err()
    );
}

#[tokio::test]
async fn ota_event_not_canceled_different_uuid() {
    let uuid = Uuid::new_v4();
    let uuid_2 = Uuid::new_v4();

    let state_mock = MockStateRepository::<PersistentState>::new();
    let system_update = MockSystemUpdate::new();

    let mut ota_req_map = HashMap::new();
    ota_req_map.insert(
        "uuid".to_owned(),
        AstarteType::String(uuid.clone().to_string()),
    );
    ota_req_map.insert(
        "operation".to_string(),
        AstarteType::String("Cancel".to_string()),
    );

    let mut pub_sub = MockPubSub::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusCode.eq("InternalError")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event
                    .message
                    .eq("Unable to cancel OTA request, they have different identifier")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let (ota, _dir) =
        Ota::mock_new_with_path(system_update, state_mock, "calcelled_different_uuid");
    *ota.ota_status.write().await = OtaStatus::Deployed(OtaRequest {
        uuid: uuid_2,
        url: "".to_string(),
    });
    let ota_handler = OtaHandler::mock_new_with_ota(ota);

    let result = ota_handler.ota_event(&pub_sub, ota_req_map).await;

    assert!(
        result.is_ok(),
        "expected ota event to be Ok, but got: {}",
        result.unwrap_err()
    );
}

#[tokio::test]
async fn ensure_pending_ota_ota_is_done_fail() {
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

    let mut pub_sub = MockPubSub::new();
    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Failure")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
                && ota_event.statusCode.eq("SystemRollback")
                && ota_event.message.eq("Unable to switch slot")
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

    let ota_handler = OtaHandler::mock_new(system_update, state_mock);
    let result = ota_handler.ensure_pending_ota_is_done(&pub_sub).await;
    assert!(result.is_err());

    if let DeviceManagerError::Ota(ota_error) = result.err().unwrap() {
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
async fn ensure_pending_ota_is_done_ota_success() {
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

    let mut pub_sub = MockPubSub::new();
    let mut seq = mockall::Sequence::new();

    pub_sub
        .expect_send_object()
        .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
            ota_event.status.eq("Success")
                && ota_event.statusCode.eq("")
                && ota_event.statusProgress == 0
                && ota_event.requestUUID == uuid.to_string()
        })
        .once()
        .returning(|_: &str, _: &str, _: OtaEvent| Ok(()))
        .in_sequence(&mut seq);

    let ota_handler = OtaHandler::mock_new(system_update, state_mock);
    let result = ota_handler.ensure_pending_ota_is_done(&pub_sub).await;

    assert!(result.is_ok());
}
