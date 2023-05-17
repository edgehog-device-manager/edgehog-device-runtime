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
    use tempdir::TempDir;
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;

    use crate::data::MockPublisher;
    use crate::error::DeviceManagerError;
    use crate::ota::ota_handler::{
        run_ota, wget, Ota, OtaError, OtaEvent, OtaHandler, OtaRequest, OtaStatus, PersistentState,
    };
    use crate::ota::rauc::BundleInfo;
    use crate::ota::MockSystemUpdate;
    use crate::repository::MockStateRepository;

    impl Default for OtaRequest {
        fn default() -> Self {
            OtaRequest {
                uuid: Uuid::new_v4(),
                url: "http://ota.bin".to_string(),
            }
        }
    }

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir() -> (TempDir, String) {
        let dir = TempDir::new("edgehog").unwrap();
        let str = dir.path().to_str().unwrap().to_string();

        (dir, str)
    }

    impl<'a> Ota<'a> {
        fn mock_new(
            system_update: MockSystemUpdate,
            state_mock: MockStateRepository<PersistentState>,
        ) -> Self {
            Ota {
                system_update: Box::new(system_update),
                state_repository: Box::new(state_mock),
                download_file_path: "".to_owned(),
                ota_status: Arc::new(RwLock::new(OtaStatus::Idle)),
            }
        }

        fn mock_new_channel(
            system_update: MockSystemUpdate,
            state_mock: MockStateRepository<PersistentState>,
        ) -> Self {
            Ota {
                system_update: Box::new(system_update),
                state_repository: Box::new(state_mock),
                download_file_path: "".to_owned(),
                ota_status: Arc::new(RwLock::new(OtaStatus::Idle)),
            }
        }
    }

    impl OtaHandler {
        async fn mock_new(
            system_update: MockSystemUpdate,
            state_mock: MockStateRepository<PersistentState>,
        ) -> Result<Self, DeviceManagerError> {
            let (sender, receiver) = mpsc::channel(8);
            let ota = Ota::mock_new_channel(system_update, state_mock);
            tokio::spawn(run_ota(ota, receiver));

            Ok(Self {
                sender,
                ota_cancellation: Arc::new(RwLock::new(None)),
            })
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Init_to_OtaStatusMessage() {
        let expected_ota_event = OtaEvent {
            requestUUID: "".to_string(),
            status: "".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Init);
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Acknowledged_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Acknowledged".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event: OtaEvent = OtaEvent::from(&OtaStatus::Acknowledged(ota_request));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID)
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_downloading_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Downloading".to_string(),
            statusProgress: 100,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Downloading(ota_request, 100));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
        assert_eq!(expected_ota_event.statusProgress, ota_event.statusProgress);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deploying_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deploying".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Deploying(ota_request));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deployed_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deployed".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Deployed(ota_request));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Rebooting_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Rebooting".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Rebooting(ota_request));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Success_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Success".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Success(ota_request));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Error_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Error".to_string(),
            statusProgress: 0,
            statusCode: "RequestError".to_string(),
            message: "Invalid data".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Error(
            OtaError::Request("Invalid data"),
            ota_request,
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_RequestError_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "RequestError".to_string(),
            message: "Invalid data".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::Request("Invalid data"),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_NetworkError_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "NetworkError".to_string(),
            message: "no network".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::Network("no network".to_string()),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_IOError_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "IOError".to_string(),
            message: "Invalid path".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::IO("Invalid path".to_string()),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_Internal_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "InternalError".to_string(),
            message: "system damage".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::Internal("system damage"),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_InvalidBaseImage_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "InvalidBaseImage".to_string(),
            message: "Unable to get info from ota".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::InvalidBaseImage("Unable to get info from ota".to_string()),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_SystemRollback_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "SystemRollback".to_string(),
            message: "Unable to switch partition".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Failure(
            OtaError::SystemRollback("Unable to switch partition"),
            Some(ota_request),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[tokio::test]
    async fn last_error_ok() {
        let mut system_update = MockSystemUpdate::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        system_update
            .expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let ota = Ota::mock_new(system_update, state_mock);

        let last_error_result = ota.last_error().await;

        assert!(last_error_result.is_ok());
        assert_eq!("Unable to deploy image", last_error_result.unwrap());
    }

    #[tokio::test]
    async fn last_error_fail() {
        let mut system_update = MockSystemUpdate::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        system_update.expect_last_error().returning(|| {
            Err(DeviceManagerError::FatalError(
                "Unable to call last error".to_string(),
            ))
        });

        let ota = Ota::mock_new(system_update, state_mock);

        let last_error_result = ota.last_error().await;

        assert!(last_error_result.is_err());
        assert!(matches!(
            last_error_result.err().unwrap(),
            DeviceManagerError::FatalError(_)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_empty_data() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota
            .acknowledged(&ota_status_publisher, HashMap::new())
            .await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_uuid() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        let data = HashMap::from([(
            "url".to_string(),
            AstarteType::String("http://instance.ota.bin".to_string()),
        )]);

        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.acknowledged(&ota_status_publisher, data).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_data_with_one_key() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        let data = HashMap::from([
            (
                "url".to_string(),
                AstarteType::String("http://instance.ota.bin".to_string()),
            ),
            (
                "uuid".to_string(),
                AstarteType::String("bad_uuid".to_string()),
            ),
            (
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            ),
        ]);

        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.acknowledged(&ota_status_publisher, data).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn ota_event_fail_data_with_wrong_astarte_type() {
        let system_update = MockSystemUpdate::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut data = HashMap::new();
        data.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        data.insert("uuid".to_owned(), AstarteType::Integer(0));
        data.insert(
            "operation".to_string(),
            AstarteType::String("Update".to_string()),
        );

        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let ota_status = ota.acknowledged(&ota_status_publisher, data).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        let uuid = Uuid::new_v4();
        let data = HashMap::from([
            (
                "url".to_string(),
                AstarteType::String("http://instance.ota.bin".to_string()),
            ),
            (
                "uuid".to_string(),
                AstarteType::String(uuid.clone().to_string()),
            ),
            (
                "operation".to_string(),
                AstarteType::String("Update".to_string()),
            ),
        ]);

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.ota_status = Arc::new(RwLock::new(OtaStatus::Init));

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let ota_status = ota.acknowledged(&ota_status_publisher, data).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(ota_status_received, OtaStatus::Acknowledged(_)));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(ota_status, OtaStatus::Acknowledged(_)))
    }

    #[tokio::test]
    async fn try_to_downloading_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();
        let ota_request = OtaRequest::default();
        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.downloading(ota_request, &ota_status_publisher).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(ota_status_received, OtaStatus::Downloading(_, 0)));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(ota_status, OtaStatus::Downloading(_, _)));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_request() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_info().returning(|_: &str| {
            Err(DeviceManagerError::FatalError(
                "Unable to get info".to_string(),
            ))
        });

        let mut ota_request = OtaRequest::default();
        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        ota_request.url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_5_wget() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        let mut ota_request = OtaRequest::default();

        let server = MockServer::start();
        ota_request.url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(404);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(4);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert_hits(5);

        for _ in 0..4 {
            let receive_result = ota_status_receiver.try_recv();
            assert!(receive_result.is_ok());
            let ota_status_received = receive_result.unwrap();
            assert!(matches!(
                ota_status_received,
                OtaStatus::Error(OtaError::Network(_), _)
            ));
        }

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Network(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_info() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_info().returning(|_: &str| {
            Err(DeviceManagerError::FatalError(
                "Unable to get info".to_string(),
            ))
        });

        let mut ota_request = OtaRequest::default();
        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        ota_request.url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_call_compatible() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Err(DeviceManagerError::FatalError("empty value".to_string())));

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let mut ota_request = OtaRequest::default();
        let server = MockServer::start();
        ota_request.url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_compatible() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        system_update
            .expect_compatible()
            .returning(|| Ok("rauc-demo-arm".to_string()));

        let mut ota_request = OtaRequest::default();

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        ota_request.url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_call_boot_slot() {
        let state_mock = MockStateRepository::<PersistentState>::new();
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

        system_update.expect_boot_slot().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let mut ota_request = OtaRequest::default();
        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        ota_request.url = ota_url;
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_write_state() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        state_mock.expect_write().returning(|_| {
            Err(DeviceManagerError::FatalError(
                "Unable to write".to_string(),
            ))
        });

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
            .returning(|| Ok("A".to_string()));

        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();
        let mut ota_request = OtaRequest::default();
        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        ota_request.url = ota_url;

        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        assert!(matches!(ota_status, OtaStatus::Failure(OtaError::IO(_), _)));
    }

    #[tokio::test]
    async fn try_to_deploying_success() {
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
            .expect_boot_slot()
            .returning(|| Ok("A".to_string()));

        let mut ota_request = OtaRequest::default();
        let binary_content = b"\x80\x02\x03";
        let binary_size = binary_content.len();

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        ota_request.url = ota_url;
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", binary_size.to_string())
                .body(binary_content);
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(2);

        let ota_status = ota.deploying(ota_request, &ota_status_publisher).await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(ota_status_received, OtaStatus::Deploying(_)));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(ota_status, OtaStatus::Deploying(_)));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_install_bundle() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update
            .expect_install_bundle()
            .returning(|_| Err(DeviceManagerError::FatalError("install fail".to_string())));

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota
            .deployed(OtaRequest::default(), &ota_status_publisher)
            .await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_operation() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_install_bundle().returning(|_| Ok(()));
        system_update.expect_operation().returning(|| {
            Err(DeviceManagerError::FatalError(
                "operation call fail".to_string(),
            ))
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota
            .deployed(OtaRequest::default(), &ota_status_publisher)
            .await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_receive_completed() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_install_bundle().returning(|_| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| {
            Err(DeviceManagerError::FatalError(
                "receive_completed call fail".to_string(),
            ))
        });

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota
            .deployed(OtaRequest::default(), &ota_status_publisher)
            .await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_signal() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_install_bundle().returning(|_| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update
            .expect_receive_completed()
            .returning(|| Ok(-1));
        system_update
            .expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, _) = mpsc::channel(1);

        let ota_status = ota
            .deployed(OtaRequest::default(), &ota_status_publisher)
            .await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update.expect_install_bundle().returning(|_| Ok(()));
        system_update
            .expect_operation()
            .returning(|| Ok("".to_string()));
        system_update.expect_receive_completed().returning(|| Ok(0));

        let ota_request = OtaRequest::default();

        let mut ota = Ota::mock_new(system_update, state_mock);
        ota.download_file_path = "/tmp".to_string();
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let ota_status = ota.deployed(ota_request, &ota_status_publisher).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(ota_status_received, OtaStatus::Deployed(_)));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(ota_status, OtaStatus::Deployed(_)));
    }

    #[tokio::test]
    async fn try_to_rebooting_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();
        let ota_request = OtaRequest::default();

        let ota = Ota::mock_new(system_update, state_mock);
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let ota_status = ota.rebooting(ota_request, &ota_status_publisher).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(ota_status_received, OtaStatus::Rebooting(_)));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(ota_status, OtaStatus::Rebooted));
    }

    #[tokio::test]
    async fn try_to_success_no_pending_update() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        state_mock.expect_exists().returning(|| false);

        let ota = Ota::mock_new(system_update, state_mock);
        let ota_status = ota.success().await;

        assert!(matches!(ota_status, OtaStatus::NoPendingOta));
    }

    #[tokio::test]
    async fn try_to_success_fail_read_state() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        state_mock.expect_exists().returning(|| true);
        state_mock
            .expect_read()
            .returning(move || Err(DeviceManagerError::FatalError("Unable to read".to_string())));

        let ota = Ota::mock_new(system_update, state_mock);
        let ota_status = ota.success().await;

        assert!(matches!(ota_status, OtaStatus::Failure(OtaError::IO(_), _)));
    }

    #[tokio::test]
    async fn try_to_success_fail_pending_ota() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_clear().returning(|| Ok(()));

        system_update
            .expect_boot_slot()
            .returning(|| Ok("A".to_owned()));

        let ota = Ota::mock_new(system_update, state_mock);
        let ota_status = ota.success().await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::SystemRollback(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_success() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_clear().returning(|| Ok(()));

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

        let ota = Ota::mock_new(system_update, state_mock);
        let ota_status = ota.success().await;

        assert!(matches!(ota_status, OtaStatus::Success(_)));
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

    #[tokio::test]
    async fn do_pending_ota_fail_boot_slot() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });

        let mut system_update = MockSystemUpdate::new();
        system_update.expect_boot_slot().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let ota = Ota::mock_new(system_update, state_mock);

        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;

        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Internal(_),));
    }

    #[tokio::test]
    async fn do_pending_ota_fail_switch_slot() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });

        let mut system_update = MockSystemUpdate::new();
        system_update
            .expect_boot_slot()
            .returning(|| Ok("A".to_owned()));

        let ota = Ota::mock_new(system_update, state_mock);

        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;

        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::SystemRollback(_),));
    }

    #[tokio::test]
    async fn do_pending_ota_fail_get_primary() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid,
                slot: slot.to_owned(),
            })
        });

        let mut system_update = MockSystemUpdate::new();
        system_update
            .expect_boot_slot()
            .returning(|| Ok("B".to_owned()));
        system_update.expect_get_primary().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let ota = Ota::mock_new(system_update, state_mock);
        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;

        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Internal(_),));
    }

    #[tokio::test]
    async fn do_pending_ota_mark_slot_fail() {
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
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Err(DeviceManagerError::FatalError(
                "Unable to call mark function".to_string(),
            ))
        });

        let ota = Ota::mock_new(system_update, state_mock);

        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Internal(_),));
    }

    #[tokio::test]
    async fn do_pending_ota_fail_marked_wrong_slot() {
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
            .returning(|| Ok("B".to_owned()));
        system_update
            .expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        system_update.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.1".to_owned(),
                "marked slot rootfs.1 as good".to_owned(),
            ))
        });

        let ota = Ota::mock_new(system_update, state_mock);

        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Internal(_),));
    }

    #[tokio::test]
    async fn do_pending_ota_success() {
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

        let ota = Ota::mock_new(system_update, state_mock);

        let state = ota.state_repository.read().unwrap();
        let result = ota.do_pending_ota(&state).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn wget_failed() {
        let (_dir, t_dir) = temp_dir();

        let server = MockServer::start();
        let hello_mock = server.mock(|when, then| {
            when.method(GET);
            then.status(500);
        });

        let ota_file = format!("{}/ota,bin", t_dir);
        let (ota_status_publisher, _) = mpsc::channel(1);

        let result = wget(
            server.url("/ota.bin").as_str(),
            ota_file.as_str(),
            &Uuid::new_v4(),
            &ota_status_publisher,
        )
        .await;

        hello_mock.assert();
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Network(_),));
    }

    #[tokio::test]
    async fn wget_failed_wrong_content_length() {
        let (_dir, t_dir) = temp_dir();

        let binary_content = b"\x80\x02\x03";

        let server = MockServer::start();
        let ota_url = server.url("/ota.bin");
        let mock_ota_file_request = &server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200)
                .header("content-Length", 0.to_string())
                .body(binary_content);
        });

        let ota_file = format!("{}/ota.bin", t_dir);
        let uuid_request = Uuid::new_v4();

        let (ota_status_publisher, _) = mpsc::channel(1);

        let result = wget(
            ota_url.as_str(),
            ota_file.as_str(),
            &uuid_request,
            &ota_status_publisher,
        )
        .await;

        mock_ota_file_request.assert();
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Network(_),));
    }

    #[tokio::test]
    async fn wget_with_empty_payload() {
        let (_dir, t_dir) = temp_dir();

        let server = MockServer::start();
        let mock_ota_file_request = server.mock(|when, then| {
            when.method(GET).path("/ota.bin");
            then.status(200).body(b"");
        });

        let ota_file = format!("{}/ota.bin", t_dir);
        let (ota_status_publisher, _) = mpsc::channel(1);

        let result = wget(
            server.url("/ota.bin").as_str(),
            ota_file.as_str(),
            &Uuid::new_v4(),
            &ota_status_publisher,
        )
        .await;

        mock_ota_file_request.assert();
        assert!(result.is_err());
        assert!(matches!(result.err().unwrap(), OtaError::Network(_),));
    }

    #[tokio::test]
    async fn wget_success() {
        let (_dir, t_dir) = temp_dir();

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

        let ota_file = format!("{}/ota.bin", t_dir);
        let uuid_request = Uuid::new_v4();

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);

        let result = wget(
            ota_url.as_str(),
            ota_file.as_str(),
            &uuid_request,
            &ota_status_publisher,
        )
        .await;
        mock_ota_file_request.assert();

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_ok());
        let ota_status_received = receive_result.unwrap();
        assert!(matches!(
            ota_status_received,
            OtaStatus::Downloading(_, 100)
        ));

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(result.is_ok());
    }
}
