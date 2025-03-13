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

use std::fmt::Debug;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use astarte_device_sdk::aggregate::AstarteObject;
use astarte_device_sdk::{Client, IntoAstarteObject};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::controller::actor::Actor;
use crate::error::DeviceManagerError;
use crate::ota::rauc::OTARauc;
use crate::ota::OtaError;
use crate::ota::{Ota, OtaId, OtaStatus};
use crate::repository::file_state_repository::FileStateRepository;

use super::event::{OtaOperation, OtaRequest};
use super::PersistentState;

const MAX_OTA_OPERATION: usize = 2;

#[derive(Debug, Clone, IntoAstarteObject)]
#[allow(non_snake_case)]
pub struct OtaEvent {
    pub requestUUID: String,
    pub status: String,
    pub statusProgress: i32,
    pub statusCode: String,
    pub message: String,
}

pub struct OtaStatusMessage {
    pub status_code: String,
    pub message: String,
}

/// Message passed to the OTA thread to start the upgrade
#[derive(Debug, Clone)]
pub struct OtaMessage {
    pub ota_id: OtaId,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Default)]
pub struct OtaInProgress(Arc<AtomicBool>);

impl OtaInProgress {
    pub fn in_progress(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn set_in_progress(&self, in_progress: bool) {
        self.0
            .store(in_progress, std::sync::atomic::Ordering::Release);
    }
}

/// Provides the communication with Ota.
#[derive(Debug)]
pub struct OtaHandler {
    pub ota_tx: mpsc::Sender<OtaMessage>,
    pub publisher_tx: mpsc::Sender<OtaStatus>,
    pub flag: OtaInProgress,
    pub current: Option<OtaMessage>,
}

impl OtaHandler {
    pub async fn start<C>(
        tasks: &mut JoinSet<stable_eyre::Result<()>>,
        client: C,
        opts: &crate::DeviceManagerOptions,
    ) -> Result<Self, DeviceManagerError>
    where
        C: Client + Send + Sync + 'static,
    {
        let (publisher_tx, publisher_rx) = mpsc::channel(8);
        let (ota_tx, ota_rx) = mpsc::channel(MAX_OTA_OPERATION);

        let system_update = OTARauc::connect().await?;

        let state_repository = FileStateRepository::new(&opts.store_directory, "state.json");

        let publisher = OtaPublisher::new(client);

        let flag = OtaInProgress::default();

        let ota = Ota::<OTARauc, FileStateRepository<PersistentState>>::new(
            publisher_tx.clone(),
            flag.clone(),
            system_update,
            state_repository,
        )?;

        tasks.spawn(publisher.spawn(publisher_rx));
        tasks.spawn(ota.spawn(ota_rx));

        Ok(Self {
            ota_tx,
            publisher_tx,
            flag,
            current: None,
        })
    }

    /// Checks if there is an OTA in progress
    pub fn in_progress(&self) -> bool {
        self.flag.in_progress()
    }

    pub async fn handle_event(&mut self, req: OtaRequest) -> Result<(), DeviceManagerError> {
        let operation = req.operation;
        let id = OtaId::from(req);

        match operation {
            OtaOperation::Update => self.handle_update(id).await,
            OtaOperation::Cancel => {
                self.handle_cancel(id).await;

                Ok(())
            }
        }
    }

    async fn handle_update(&mut self, id: OtaId) -> Result<(), DeviceManagerError> {
        if self.check_update_already_in_progress(&id).await {
            return Ok(());
        }

        self.flag.set_in_progress(true);

        let ota_message = OtaMessage {
            ota_id: id,
            cancel: CancellationToken::new(),
        };

        self.ota_tx.send(ota_message.clone()).await.map_err(|_| {
            OtaError::Internal("Unable to execute HandleOtaEvent, receiver channel dropped")
        })?;

        self.current = Some(ota_message);

        Ok(())
    }

    #[must_use]
    pub(super) async fn check_update_already_in_progress(&mut self, id: &OtaId) -> bool {
        // OTA no longer in progress, a failure happened
        if self.current.is_some() && !self.flag.in_progress() {
            self.current = None;

            return false;
        }

        match &self.current {
            Some(current) if current.ota_id != *id => {
                debug!(
                    "different ota id between current {} and event {}",
                    current.ota_id, id,
                );

                let _ = self
                    .publisher_tx
                    .send(OtaStatus::Failure(
                        OtaError::UpdateAlreadyInProgress,
                        Some(id.clone()),
                    ))
                    .await;

                true
            }
            // Same OTA id
            Some(current) => {
                debug!("Ota request received with same ota id: {}", current.ota_id);

                true
            }
            // In progress, but after a reboot, so it's ok to queue the next OTA
            None => false,
        }
    }

    /// Cancel an in progress OTA.
    ///
    /// This will check only the current since after a reboot we cannot cancel an OTA. The cancel
    /// will be handled by the OTA task.
    async fn handle_cancel(&mut self, id: OtaId) {
        match &self.current {
            Some(current) if current.ota_id == id => {
                info!("Ota {id} cancelled");

                current.cancel.cancel();

                self.current = None;
            }
            Some(_) => {
                let _ = self
                    .publisher_tx
                    .send(OtaStatus::Failure(
                        OtaError::Internal(
                            "Unable to cancel OTA request, they have different identifier",
                        ),
                        Some(id),
                    ))
                    .await;
            }
            None => {
                let _ = self
                    .publisher_tx
                    .send(OtaStatus::Failure(
                        OtaError::Internal(
                            "Unable to cancel OTA request, internal request is empty",
                        ),
                        Some(id),
                    ))
                    .await;
            }
        }
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
            OtaError::Io(message) => {
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
            OtaError::InconsistentState => {
                ota_status_message.status_code = "InternalError".to_string()
            }
        }

        ota_status_message
    }
}

#[derive(Debug)]
pub struct OtaPublisher<C> {
    client: C,
}

impl<C> OtaPublisher<C> {
    pub fn new(client: C) -> Self {
        Self { client }
    }

    async fn send_ota_event(&mut self, ota_status: &OtaStatus) -> Result<(), OtaError>
    where
        C: Client + Send + Sync,
    {
        let Some(ota_event) = ota_status.as_event() else {
            return Ok(());
        };

        debug!("Sending ota response {:?}", ota_event);

        let data = AstarteObject::try_from(ota_event).map_err(|err| {
            error!(
                error = format!("{:#}", eyre::Report::new(err)),
                "invalid ota_event"
            );

            OtaError::Internal("couldn't convert ota_event to Astarte object")
        })?;

        self.client
            .send_object("io.edgehog.devicemanager.OTAEvent", "/event", data)
            .await
            .map_err(|error| {
                let message = "Unable to publish ota_event".to_string();
                error!("{message} : {error}");
                OtaError::Network(message)
            })?;

        Ok(())
    }
}

#[async_trait]
impl<C> Actor for OtaPublisher<C>
where
    C: Client + Send + Sync,
{
    type Msg = OtaStatus;

    fn task() -> &'static str {
        "ota-publisher"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        if let Err(err) = self.send_ota_event(&msg).await {
            error!(
                error = format!("{:#}", stable_eyre::Report::new(err)),
                "couldn't send ota event"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::ota::ota_handler::OtaEvent;
    use crate::ota::{DeployProgress, OtaError};
    use crate::ota::{OtaId, OtaStatus};

    use uuid::Uuid;

    impl Default for OtaId {
        fn default() -> Self {
            OtaId {
                uuid: Uuid::new_v4(),
                url: "http://ota.bin".to_string(),
            }
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Init_to_OtaStatusMessage() {
        let uuid = uuid::Uuid::new_v4();

        let ota_event = OtaStatus::Init(OtaId {
            uuid,
            url: "".to_string(),
        })
        .as_event();

        assert!(ota_event.is_none());
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Acknowledged_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Acknowledged".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event: OtaEvent = OtaStatus::Acknowledged(ota_request).as_event().unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID)
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deploying_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deploying".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaStatus::Deploying(ota_request, DeployProgress::default())
            .as_event()
            .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deploying_100_done_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deploying".to_string(),
            statusProgress: 100,
            statusCode: "".to_string(),
            message: "done".to_string(),
        };

        let ota_event = OtaStatus::Deploying(
            ota_request,
            DeployProgress {
                percentage: 100,
                message: "done".to_string(),
            },
        )
        .as_event()
        .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Deployed_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Deployed".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaStatus::Deployed(ota_request).as_event().unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Rebooting_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Rebooting".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaStatus::Rebooting(ota_request).as_event().unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Success_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Success".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        let ota_event = OtaStatus::Success(ota_request).as_event().unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_RequestError_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "RequestError".to_string(),
            message: "Invalid data".to_string(),
        };

        let ota_event = OtaStatus::Failure(OtaError::Request("Invalid data"), Some(ota_request))
            .as_event()
            .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_NetworkError_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "NetworkError".to_string(),
            message: "no network".to_string(),
        };

        let ota_event = OtaStatus::Failure(
            OtaError::Network("no network".to_string()),
            Some(ota_request),
        )
        .as_event()
        .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_IOError_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "IOError".to_string(),
            message: "Invalid path".to_string(),
        };

        let ota_event =
            OtaStatus::Failure(OtaError::Io("Invalid path".to_string()), Some(ota_request))
                .as_event()
                .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_Internal_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "InternalError".to_string(),
            message: "system damage".to_string(),
        };

        let ota_event = OtaStatus::Failure(OtaError::Internal("system damage"), Some(ota_request))
            .as_event()
            .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_InvalidBaseImage_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "InvalidBaseImage".to_string(),
            message: "Unable to get info from ota".to_string(),
        };

        let ota_event = OtaStatus::Failure(
            OtaError::InvalidBaseImage("Unable to get info from ota".to_string()),
            Some(ota_request),
        )
        .as_event()
        .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[test]
    #[allow(non_snake_case)]
    fn convert_ota_status_Failure_SystemRollback_to_OtaStatusMessage() {
        let ota_request = OtaId::default();
        let expected_ota_event = OtaEvent {
            requestUUID: ota_request.uuid.to_string(),
            status: "Failure".to_string(),
            statusProgress: 0,
            statusCode: "SystemRollback".to_string(),
            message: "Unable to switch partition".to_string(),
        };

        let ota_event = OtaStatus::Failure(
            OtaError::SystemRollback("Unable to switch partition"),
            Some(ota_request),
        )
        .as_event()
        .unwrap();
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }
}
