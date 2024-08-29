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

use std::collections::HashMap;
use std::fmt::Debug;
use std::str::FromStr;
use std::sync::Arc;

use astarte_device_sdk::types::AstarteType;
use astarte_device_sdk::AstarteAggregate;
use log::{debug, error};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::data::Publisher;
use crate::error::DeviceManagerError;
use crate::ota::ota_handle::{Ota, OtaMessage, OtaRequest, OtaStatus};
use crate::ota::rauc::OTARauc;
use crate::ota::OtaError;
use crate::repository::file_state_repository::FileStateRepository;

use super::ota_handle::PersistentState;

enum OtaOperation {
    Cancel,
    Update,
}

#[derive(AstarteAggregate, Debug)]
#[allow(non_snake_case)]
pub struct OtaEvent {
    pub requestUUID: String,
    pub status: String,
    pub statusProgress: i32,
    pub statusCode: String,
    pub message: String,
}

struct OtaStatusMessage {
    status_code: String,
    message: String,
}

/// Provides the communication with Ota.
#[derive(Clone)]
pub struct OtaHandler {
    pub sender: mpsc::Sender<OtaMessage>,
    pub ota_cancellation: Arc<RwLock<Option<CancellationToken>>>,
}

impl FromStr for OtaOperation {
    type Err = ();

    fn from_str(s: &str) -> Result<OtaOperation, ()> {
        match s {
            "Cancel" => Ok(OtaOperation::Cancel),
            "Update" => Ok(OtaOperation::Update),
            _ => Err(()),
        }
    }
}

impl OtaHandler {
    pub async fn new(opts: &crate::DeviceManagerOptions) -> Result<Self, DeviceManagerError> {
        let (sender, receiver) = mpsc::channel(8);
        let system_update = OTARauc::new().await?;

        let state_repository = FileStateRepository::new(&opts.store_directory, "state.json");

        let ota = Ota::<OTARauc, FileStateRepository<PersistentState>>::new(
            opts,
            system_update,
            state_repository,
        )
        .await?;
        tokio::spawn(crate::ota::ota_handle::run_ota(ota, receiver));

        Ok(Self {
            sender,
            ota_cancellation: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn ensure_pending_ota_is_done<P>(&self, sdk: &P) -> Result<(), DeviceManagerError>
    where
        P: Publisher + Send + Sync,
    {
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(8);
        let msg = OtaMessage::EnsurePendingOta {
            respond_to: ota_status_publisher,
        };

        if self.sender.send(msg).await.is_err() {
            return Err(DeviceManagerError::Ota(OtaError::Internal(
                "Unable to execute EnsurePendingOta, receiver channel dropped",
            )));
        }

        while let Some(ota_status) = ota_status_receiver.recv().await {
            send_ota_event(sdk, &ota_status).await?;

            if let OtaStatus::Failure(ota_error, _) = ota_status {
                return Err(DeviceManagerError::Ota(ota_error));
            }
        }

        Ok(())
    }

    async fn get_ota_status(&self) -> Result<OtaStatus, DeviceManagerError> {
        let (ota_status_publisher, ota_status_receiver) = oneshot::channel();
        let msg = OtaMessage::GetOtaStatus {
            respond_to: ota_status_publisher,
        };

        self.sender.send(msg).await.map_err(|_| {
            DeviceManagerError::Ota(OtaError::Internal(
                "Unable to get the ota status, receiver channel dropped",
            ))
        })?;

        ota_status_receiver.await.map_err(|_| {
            DeviceManagerError::Ota(OtaError::Internal("Unable to get the ota status"))
        })
    }

    pub async fn ota_event<P>(
        &self,
        sdk: &P,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError>
    where
        P: Publisher + Send + Sync,
    {
        let Some(AstarteType::String(operation_str)) = data.get("operation") else {
            error!("missing ota operation: {:?}", data);

            return Err(DeviceManagerError::Ota(OtaError::Request(
                "Ota operation unsupported",
            )));
        };

        match operation_str.parse() {
            Ok(OtaOperation::Update) => self.handle_update(sdk, data).await,
            Ok(OtaOperation::Cancel) => self.handle_cancel(sdk, data).await,
            Err(()) => {
                error!("could not parse operation: {}", operation_str);

                Err(DeviceManagerError::Ota(OtaError::Request(
                    "Ota operation unsupported",
                )))
            }
        }
    }

    async fn handle_update<P>(
        &self,
        sdk: &P,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError>
    where
        P: Publisher + Send + Sync,
    {
        let Some(AstarteType::String(operation_str)) = data.get("uuid") else {
            error!("update data missing uuid: {:?}", data);

            return Ok(());
        };

        let uuid = Uuid::parse_str(operation_str).map_err(|_| {
            DeviceManagerError::Ota(OtaError::Request("Unable to parse request_uuid"))
        })?;

        self.check_update_already_in_progress(uuid, sdk).await?;

        let mut ota_status_receiver = self.start_ota_update(data).await?;

        while let Some(ota_status) = ota_status_receiver.recv().await {
            send_ota_event(sdk, &ota_status).await?;

            //After entering in Deploying state the OTA cannot be stopped.
            if let OtaStatus::Deploying(_, _) = &ota_status {
                *self.ota_cancellation.write().await = None;
            } else if let OtaStatus::Failure(ota_error, _) = ota_status {
                *self.ota_cancellation.write().await = None;
                return Err(DeviceManagerError::Ota(ota_error));
            }
        }
        Ok(())
    }

    /// Sends the cancellation token and channel to start the update process.
    pub(crate) async fn start_ota_update(
        &self,
        data: HashMap<String, AstarteType>,
    ) -> Result<mpsc::Receiver<OtaStatus>, DeviceManagerError> {
        let (ota_status_publisher, ota_status_receiver) = mpsc::channel(8);

        let cancel_token = CancellationToken::new();
        *self.ota_cancellation.write().await = Some(cancel_token.clone());

        let msg = OtaMessage::HandleOtaEvent {
            data,
            cancel_token,
            respond_to: ota_status_publisher,
        };

        self.sender.send(msg).await.map_err(|_| {
            DeviceManagerError::Ota(OtaError::Internal(
                "Unable to execute HandleOtaEvent, receiver channel dropped",
            ))
        })?;

        Ok(ota_status_receiver)
    }

    async fn check_update_already_in_progress<P>(
        &self,
        uuid: Uuid,
        sdk: &P,
    ) -> Result<(), DeviceManagerError>
    where
        P: Publisher + Send + Sync,
    {
        match self.get_ota_status().await {
            Err(_) => Ok(()),
            Ok(OtaStatus::Idle) => Ok(()),
            Ok(ota_status) => {
                match ota_status.ota_request() {
                    Some(current_ota_request) if current_ota_request.uuid == uuid => {
                        // Send the current ota status
                        let _ = send_ota_event(sdk, &ota_status).await;
                    }
                    _ => {
                        let _ = send_ota_event(
                            sdk,
                            &OtaStatus::Failure(
                                OtaError::UpdateAlreadyInProgress,
                                Some(OtaRequest {
                                    uuid,
                                    url: "".to_string(),
                                }),
                            ),
                        )
                        .await;
                    }
                }
                Err(DeviceManagerError::Ota(OtaError::UpdateAlreadyInProgress))
            }
        }
    }

    async fn handle_cancel<P>(
        &self,
        sdk: &P,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError>
    where
        P: Publisher + Send + Sync,
    {
        let Some(AstarteType::String(request_uuid_str)) = &data.get("uuid") else {
            return Err(DeviceManagerError::Ota(OtaError::Request(
                "Missing uuid in cancel request data",
            )));
        };

        let request_uuid = Uuid::parse_str(request_uuid_str).map_err(|_| {
            DeviceManagerError::Ota(OtaError::Request("Unable to parse request_uuid"))
        })?;

        let cancel_ota_request = OtaRequest {
            uuid: request_uuid,
            url: "".to_string(),
        };

        let ota_status = match self.get_ota_status().await {
            Ok(ota_status) => ota_status,
            Err(err) => {
                let message = "Unable to cancel OTA request";
                error!("{message} : {err}");
                send_ota_event(
                    sdk,
                    &OtaStatus::Failure(OtaError::Internal(message), Some(cancel_ota_request)),
                )
                .await?;

                return Ok(());
            }
        };

        match ota_status.ota_request() {
            None => {
                send_ota_event(
                    sdk,
                    &OtaStatus::Failure(
                        OtaError::Internal(
                            "Unable to cancel OTA request, internal request is empty",
                        ),
                        Some(cancel_ota_request),
                    ),
                )
                .await?;
            }
            Some(current_ota_request) if cancel_ota_request.uuid != current_ota_request.uuid => {
                send_ota_event(
                    sdk,
                    &OtaStatus::Failure(
                        OtaError::Internal(
                            "Unable to cancel OTA request, they have different identifier",
                        ),
                        Some(cancel_ota_request),
                    ),
                )
                .await?;
            }
            _ => {
                let mut ota_cancellation = self.ota_cancellation.write().await;
                if let Some(ota_token) = ota_cancellation.take() {
                    ota_token.cancel();
                    send_ota_event(
                        sdk,
                        &OtaStatus::Failure(OtaError::Canceled, Some(cancel_ota_request)),
                    )
                    .await?;
                } else {
                    send_ota_event(
                        sdk,
                        &OtaStatus::Failure(
                            OtaError::Internal("Unable to cancel OTA request"),
                            Some(cancel_ota_request),
                        ),
                    )
                    .await?
                }
            }
        }

        Ok(())
    }
}

impl From<&OtaStatus> for OtaEvent {
    fn from(ota_status: &OtaStatus) -> Self {
        let mut ota_event = OtaEvent {
            requestUUID: "".to_string(),
            status: "".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        match ota_status {
            OtaStatus::Acknowledged(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Acknowledged".to_string();
            }
            OtaStatus::Downloading(ota_request, progress) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.statusProgress = *progress;
                ota_event.status = "Downloading".to_string();
            }
            OtaStatus::Deploying(ota_request, deploying_progress) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Deploying".to_string();
                ota_event.statusProgress = deploying_progress.percentage;
                ota_event.message = deploying_progress.clone().message;
            }
            OtaStatus::Deployed(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Deployed".to_string();
            }
            OtaStatus::Rebooting(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Rebooting".to_string()
            }
            OtaStatus::Success(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Success".to_string();
            }
            OtaStatus::Failure(ota_error, ota_request) => {
                if let Some(ota_request) = ota_request {
                    ota_event.requestUUID = ota_request.uuid.to_string();
                }
                ota_event.status = "Failure".to_string();
                let ota_status_message = OtaStatusMessage::from(ota_error);
                ota_event.statusCode = ota_status_message.status_code;
                ota_event.message = ota_status_message.message;
            }
            OtaStatus::Error(ota_error, ota_request) => {
                ota_event.status = "Error".to_string();
                ota_event.requestUUID = ota_request.uuid.to_string();
                let ota_status_message = OtaStatusMessage::from(ota_error);
                ota_event.statusCode = ota_status_message.status_code;
                ota_event.message = ota_status_message.message;
            }
            OtaStatus::Idle | OtaStatus::Init | OtaStatus::NoPendingOta | OtaStatus::Rebooted => {}
        }
        ota_event
    }
}

impl From<&OtaError> for OtaStatusMessage {
    fn from(ota_error: &OtaError) -> Self {
        let mut ota_status_message = OtaStatusMessage {
            status_code: "".to_string(),
            message: "".to_string(),
        };

        match ota_error {
            OtaError::Request(message) => {
                ota_status_message.status_code = "RequestError".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::UpdateAlreadyInProgress => {
                ota_status_message.status_code = "UpdateAlreadyInProgress".to_string()
            }
            OtaError::Network(message) => {
                ota_status_message.status_code = "NetworkError".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::IO(message) => {
                ota_status_message.status_code = "IOError".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::Internal(message) => {
                ota_status_message.status_code = "InternalError".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::InvalidBaseImage(message) => {
                ota_status_message.status_code = "InvalidBaseImage".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::SystemRollback(message) => {
                ota_status_message.status_code = "SystemRollback".to_string();
                ota_status_message.message = message.to_string()
            }
            OtaError::Canceled => ota_status_message.status_code = "Canceled".to_string(),
        }

        ota_status_message
    }
}

async fn send_ota_event<P>(sdk: &P, ota_status: &OtaStatus) -> Result<(), OtaError>
where
    P: Publisher + Send + Sync,
{
    if ota_status.ota_request().is_none() {
        return Ok(());
    }

    let ota_event = OtaEvent::from(ota_status);
    debug!("Sending ota response {:?}", ota_event);

    if ota_event.requestUUID.is_empty() {
        return Err(OtaError::Internal(
            "Unable to publish ota_event: request_uuid is empty",
        ));
    }

    sdk.send_object("io.edgehog.devicemanager.OTAEvent", "/event", ota_event)
        .await
        .map_err(|error| {
            let message = "Unable to publish ota_event".to_string();
            error!("{message} : {error}");
            OtaError::Network(message)
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::ota::ota_handle::{OtaRequest, OtaStatus};
    use crate::ota::ota_handler::OtaEvent;
    use crate::ota::{DeployProgress, OtaError};
    use uuid::Uuid;

    impl Default for OtaRequest {
        fn default() -> Self {
            OtaRequest {
                uuid: Uuid::new_v4(),
                url: "http://ota.bin".to_string(),
            }
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

        let ota_event = OtaEvent::from(&OtaStatus::Deploying(
            ota_request,
            DeployProgress::default(),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deploying_100_done_to_OtaStatusMessage() {
        let ota_request = OtaRequest::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deploying".to_string(),
            statusProgress: 100,
            statusCode: "".to_string(),
            message: "done".to_string(),
        };

        let ota_event = OtaEvent::from(&OtaStatus::Deploying(
            ota_request,
            DeployProgress {
                percentage: 100,
                message: "done".to_string(),
            },
        ));
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
}
