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

use astarte_device_sdk::AstarteAggregate;
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use astarte_device_sdk::types::AstarteType;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::data::Publisher;
use crate::error::DeviceManagerError;
use crate::ota::rauc::OTARauc;
use crate::ota::SystemUpdate;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistentState {
    pub uuid: Uuid,
    pub slot: String,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum OtaError {
    #[error("InvalidRequestError: {0}")]
    Request(&'static str),
    #[error("UpdateAlreadyInProgress")]
    UpdateAlreadyInProgress,
    #[error("NetworkError: {0}")]
    Network(String),
    #[error("IOError: {0}")]
    IO(String),
    #[error("InternalError: {0}")]
    Internal(&'static str),
    #[error("InvalidBaseImage: {0}")]
    InvalidBaseImage(String),
    #[error("SystemRollback: {0}")]
    SystemRollback(&'static str),
    #[error("Cancelled")]
    Cancelled,
}

enum OtaOperation {
    Cancel,
    Update,
}

#[derive(Clone, PartialEq, Debug)]
pub enum OtaStatus {
    Idle,
    Init,
    NoPendingOta,
    Acknowledged(OtaRequest),
    Downloading(OtaRequest, i32),
    Deploying(OtaRequest),
    Deployed(OtaRequest),
    Rebooting(OtaRequest),
    Rebooted,
    Success(OtaRequest),
    Error(OtaError, OtaRequest),
    Failure(OtaError, Option<OtaRequest>),
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

#[derive(PartialEq, Clone, Debug)]
pub struct OtaRequest {
    pub uuid: Uuid,
    pub url: String,
}

/// Provides ota resource accessibility only by talking with it.
pub struct Ota<'a> {
    pub system_update: Box<dyn SystemUpdate + 'a>,
    pub state_repository: Box<dyn StateRepository<PersistentState> + 'a>,
    pub download_file_path: String,
    pub ota_status: Arc<RwLock<OtaStatus>>,
}

/// An enum that defines the kind of messages we can send to the OtaActor.
pub enum OtaMessage {
    GetOtaStatus {
        respond_to: oneshot::Sender<OtaStatus>,
    },
    EnsurePendingOta {
        respond_to: mpsc::Sender<OtaStatus>,
    },
    HandleOtaEvent {
        data: HashMap<String, AstarteType>,
        cancel_token: CancellationToken,
        respond_to: mpsc::Sender<OtaStatus>,
    },
}

/// Provides the communication with Ota.
#[derive(Clone)]
pub struct OtaHandler {
    pub sender: mpsc::Sender<OtaMessage>,
    pub ota_cancellation: Arc<RwLock<Option<CancellationToken>>>,
}

impl OtaStatus {
    fn ota_request(&self) -> Option<&OtaRequest> {
        match self {
            OtaStatus::Acknowledged(ota_request)
            | OtaStatus::Downloading(ota_request, _)
            | OtaStatus::Deploying(ota_request)
            | OtaStatus::Deployed(ota_request)
            | OtaStatus::Rebooting(ota_request)
            | OtaStatus::Success(ota_request)
            | OtaStatus::Error(_, ota_request) => Some(ota_request),
            OtaStatus::Failure(_, ota_request) => ota_request.as_ref(),
            _ => None,
        }
    }
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
        let ota = Ota::new(opts).await?;
        tokio::spawn(run_ota(ota, receiver));

        Ok(Self {
            sender,
            ota_cancellation: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn ensure_pending_ota_response(
        &self,
        sdk: &impl Publisher,
    ) -> Result<(), DeviceManagerError> {
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(8);
        let msg = OtaMessage::EnsurePendingOta {
            respond_to: ota_status_publisher,
        };

        if self.sender.send(msg).await.is_err() {
            return Err(DeviceManagerError::OtaError(OtaError::Internal(
                "Unable to execute EnsurePendingOta, receiver channel dropped",
            )));
        }

        while let Some(ota_status) = ota_status_receiver.recv().await {
            send_ota_event(sdk, &ota_status).await?;

            if let OtaStatus::Failure(ota_error, _) = ota_status {
                return Err(DeviceManagerError::OtaError(ota_error));
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
            DeviceManagerError::OtaError(OtaError::Internal(
                "Unable to get the ota status, receiver channel dropped",
            ))
        })?;

        ota_status_receiver.await.map_err(|_| {
            DeviceManagerError::OtaError(OtaError::Internal("Unable to get the ota status"))
        })
    }

    pub async fn ota_event(
        &self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        if let AstarteType::String(operation_str) = &data["operation"] {
            match operation_str.parse::<OtaOperation>() {
                Ok(OtaOperation::Update) => self.handle_update(sdk, data).await,
                Ok(OtaOperation::Cancel) => self.handle_cancel(sdk, data).await,
                Err(_) => Err(DeviceManagerError::OtaError(OtaError::Request(
                    "Ota operation unsupported",
                ))),
            }
        } else {
            Err(DeviceManagerError::OtaError(OtaError::Request(
                "Ota operation unsupported",
            )))
        }
    }

    async fn handle_update(
        &self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(8);
        if let AstarteType::String(operation_str) = &data["uuid"] {
            let uuid = Uuid::parse_str(operation_str).map_err(|_| {
                DeviceManagerError::OtaError(OtaError::Request("Unable to parse request_uuid"))
            })?;

            if let Ok(ota_status) = self.get_ota_status().await {
                if ota_status != OtaStatus::Idle {
                    if let Some(current_ota_request) = ota_status.ota_request() {
                        if current_ota_request.uuid == uuid {
                            let _ = send_ota_event(sdk, &ota_status).await;
                            return Err(DeviceManagerError::OtaError(
                                OtaError::UpdateAlreadyInProgress,
                            ));
                        }
                    }

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
                    return Err(DeviceManagerError::OtaError(
                        OtaError::UpdateAlreadyInProgress,
                    ));
                }
            }

            let cancel_token = CancellationToken::new();
            *self.ota_cancellation.write().await = Some(cancel_token.clone());
            let msg = OtaMessage::HandleOtaEvent {
                data,
                cancel_token,
                respond_to: ota_status_publisher,
            };

            if self.sender.send(msg).await.is_err() {
                return Err(DeviceManagerError::OtaError(OtaError::Internal(
                    "Unable to execute HandleOtaEvent, receiver channel dropped",
                )));
            }

            while let Some(ota_status) = ota_status_receiver.recv().await {
                send_ota_event(sdk, &ota_status).await?;

                //After entering in Deploying state the OTA cannot be stopped.
                if let OtaStatus::Deploying(_) = &ota_status {
                    *self.ota_cancellation.write().await = None;
                } else if let OtaStatus::Failure(ota_error, _) = ota_status {
                    *self.ota_cancellation.write().await = None;
                    return Err(DeviceManagerError::OtaError(ota_error));
                }
            }
        }
        Ok(())
    }

    async fn handle_cancel(
        &self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        if let AstarteType::String(request_uuid_str) = &data["uuid"] {
            let request_uuid = Uuid::parse_str(request_uuid_str).map_err(|_| {
                DeviceManagerError::OtaError(OtaError::Request("Unable to parse request_uuid"))
            })?;

            let cancel_ota_request = OtaRequest {
                uuid: request_uuid,
                url: "".to_string(),
            };

            let ota_status = match self.get_ota_status().await {
                Ok(ota_status) => ota_status,
                Err(_) => {
                    send_ota_event(
                        sdk,
                        &OtaStatus::Failure(
                            OtaError::Internal("Unable to cancel OTA request"),
                            Some(cancel_ota_request),
                        ),
                    )
                    .await?;

                    return Ok(());
                }
            };

            let current_ota_request = match ota_status.ota_request() {
                Some(ota_request) => ota_request,
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

                    return Ok(());
                }
            };

            if cancel_ota_request.uuid != current_ota_request.uuid {
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
                return Ok(());
            }

            let mut ota_cancellation = self.ota_cancellation.write().await;
            if let Some(ota_token) = ota_cancellation.take() {
                ota_token.cancel();
                send_ota_event(
                    sdk,
                    &OtaStatus::Failure(OtaError::Cancelled, Some(cancel_ota_request)),
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
        Ok(())
    }
}

pub async fn run_ota(ota: Ota<'static>, mut receiver: mpsc::Receiver<OtaMessage>) {
    let ota_handle = Arc::new(RwLock::new(ota));
    while let Some(msg) = receiver.recv().await {
        let ota_handle_cloned = ota_handle.clone();
        tokio::spawn(async move {
            let ota_guard = ota_handle_cloned.read().await;
            ota_guard.handle_message(msg).await;
        });
    }
}

impl<'a> Ota<'a> {
    async fn new(opts: &crate::DeviceManagerOptions) -> Result<Ota<'a>, DeviceManagerError> {
        let ota = OTARauc::new().await?;

        Ok(Ota {
            system_update: Box::new(ota),
            state_repository: Box::new(FileStateRepository::new(
                opts.store_directory.clone(),
                "state.json".to_owned(),
            )),
            download_file_path: opts.download_directory.clone(),
            ota_status: Arc::new(RwLock::new(OtaStatus::Idle)),
        })
    }

    async fn handle_message(&self, msg: OtaMessage) {
        match msg {
            OtaMessage::HandleOtaEvent {
                data,
                cancel_token,
                respond_to,
            } => {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        debug!("OTA update channel closed by handle");
                    }
                 ota_status = self.handle_ota_event(OtaStatus::Idle, &respond_to, data) => {
                        let _ = respond_to.send(ota_status).await;
                    }
                }
            }
            OtaMessage::EnsurePendingOta { respond_to } => {
                let ota_status = self
                    .handle_ota_event(OtaStatus::Rebooted, &respond_to, HashMap::new())
                    .await;
                let _ = respond_to.send(ota_status).await;
            }
            OtaMessage::GetOtaStatus { respond_to } => {
                let _ = respond_to.send(self.ota_status.read().await.clone());
            }
        }
    }

    pub async fn last_error(&self) -> Result<String, DeviceManagerError> {
        self.system_update.last_error().await
    }

    fn get_update_file_path(&self) -> PathBuf {
        std::path::Path::new(&self.download_file_path).join("update.bin")
    }

    pub async fn acknowledged(
        &self,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
        data: HashMap<String, AstarteType>,
    ) -> OtaStatus {
        if !data.contains_key("url") || !data.contains_key("uuid") {
            return OtaStatus::Failure(
                OtaError::Request("Unable to find data in the OTA request"),
                None,
            );
        }

        if let (AstarteType::String(request_url), AstarteType::String(request_uuid_str)) =
            (&data["url"], &data["uuid"])
        {
            let request_uuid = match Uuid::parse_str(request_uuid_str) {
                Ok(uuid) => uuid,
                Err(_) => {
                    return OtaStatus::Failure(
                        OtaError::Request("Unable to parse request_uuid"),
                        None,
                    )
                }
            };

            let ota_request = OtaRequest {
                uuid: request_uuid,
                url: request_url.to_string(),
            };

            let ack_status = OtaStatus::Acknowledged(ota_request);
            if ota_status_publisher.send(ack_status.clone()).await.is_err() {
                warn!("ota_status_publisher dropped before send ack_status")
            }
            ack_status
        } else {
            let message = "Got invalid data in OTARequest";
            error!("{message}: {:?}", data);
            OtaStatus::Failure(OtaError::Request(message), None)
        }
    }

    pub async fn downloading(
        &self,
        ota_request: OtaRequest,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
    ) -> OtaStatus {
        let downloading_status = OtaStatus::Downloading(ota_request, 0);
        if ota_status_publisher
            .send(downloading_status.clone())
            .await
            .is_err()
        {
            warn!("ota_status_publisher dropped before send downloading_status")
        }
        downloading_status
    }

    pub async fn deploying(
        &self,
        ota_request: OtaRequest,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
    ) -> OtaStatus {
        let download_file_path = self.get_update_file_path();

        let download_file_path = match download_file_path.to_str() {
            Some(path) => path,
            None => {
                return OtaStatus::Failure(
                    OtaError::IO("Wrong download file path".to_string()),
                    Some(ota_request),
                )
            }
        };

        let mut ota_download_result = wget(
            &ota_request.url,
            download_file_path,
            &ota_request.uuid,
            ota_status_publisher,
        )
        .await;
        for i in 1..5 {
            if let Err(error) = ota_download_result {
                let wait = u64::pow(2, i);
                let message = "Error downloading update".to_string();
                error!("{message}: {:?}", error);
                error!("Next attempt in {}s", wait);

                if ota_status_publisher
                    .send(OtaStatus::Error(error, ota_request.clone()))
                    .await
                    .is_err()
                {
                    warn!("ota_status_publisher dropped before send error_status")
                }

                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                ota_download_result = wget(
                    &ota_request.url,
                    download_file_path,
                    &ota_request.uuid,
                    ota_status_publisher,
                )
                .await;
            } else {
                break;
            }
        }

        if let Err(error) = ota_download_result {
            OtaStatus::Failure(error, Some(ota_request.clone()))
        } else {
            let bundle_info = self.system_update.info(download_file_path).await;
            if bundle_info.is_err() {
                let message = format!(
                    "Unable to get info from ota_file in {:?}",
                    download_file_path
                );
                error!("{message} : {}", bundle_info.unwrap_err());
                return OtaStatus::Failure(
                    OtaError::InvalidBaseImage(message),
                    Some(ota_request.clone()),
                );
            }

            let bundle_info = bundle_info.unwrap();

            debug!("bundle info: {:?}", bundle_info);

            let system_image_info = self.system_update.compatible().await;
            if system_image_info.is_err() {
                let message = "Unable to get info from current deployed image".to_string();
                error!("{message} : {}", system_image_info.unwrap_err());
                return OtaStatus::Failure(
                    OtaError::InvalidBaseImage(message),
                    Some(ota_request.clone()),
                );
            }

            let system_image_info = system_image_info.unwrap();

            if bundle_info.compatible != system_image_info {
                let message = format!(
                    "bundle {} is not compatible with system {system_image_info}",
                    bundle_info.compatible
                );
                error!("{message}");
                return OtaStatus::Failure(
                    OtaError::InvalidBaseImage(message),
                    Some(ota_request.clone()),
                );
            }

            let booted_slot = self.system_update.boot_slot().await;
            if booted_slot.is_err() {
                let message = "Unable to identify the booted slot";
                error!("{message}: {}", booted_slot.unwrap_err());
                return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
            }

            let booted_slot = booted_slot.unwrap();

            if let Err(error) = self.state_repository.write(&PersistentState {
                uuid: ota_request.clone().uuid,
                slot: booted_slot,
            }) {
                let message = "Unable to persist ota state".to_string();
                error!("{message} : {error}");
                return OtaStatus::Failure(OtaError::IO(message), Some(ota_request.clone()));
            };

            let deploying_state = OtaStatus::Deploying(ota_request.clone());
            if ota_status_publisher
                .send(deploying_state.clone())
                .await
                .is_err()
            {
                warn!("ota_status_publisher dropped before send deploying_state")
            }

            deploying_state
        }
    }

    pub async fn deployed(
        &self,
        ota_request: OtaRequest,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
    ) -> OtaStatus {
        if let Err(error) = self
            .system_update
            .install_bundle(&self.download_file_path)
            .await
        {
            let message = "Unable to install ota image".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::InvalidBaseImage(message), Some(ota_request));
        }

        debug!(
            "install_bundle done, last_error={:?}",
            self.last_error().await
        );

        if let Err(error) = self.system_update.operation().await {
            let message = "Unable to get status of ota operation";
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        info!("Waiting for signal...");
        let signal = self.system_update.receive_completed().await;
        if signal.is_err() {
            let message = "Unable to receive the install completed event";
            error!("{message} : {}", signal.unwrap_err());
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        let signal = signal.unwrap();
        info!("Completed signal! {:?}", signal);

        match signal {
            0 => {
                info!("Update successful");

                let deployed_status = OtaStatus::Deployed(ota_request.clone());
                if ota_status_publisher
                    .send(deployed_status.clone())
                    .await
                    .is_err()
                {
                    warn!("ota_status_publisher dropped before send deployed_status")
                }
                deployed_status
            }
            _ => {
                let message = format!("Update failed with signal {signal}",);
                error!("{message} : {:?}", self.last_error().await);
                OtaStatus::Failure(
                    OtaError::InvalidBaseImage(message),
                    Some(ota_request.clone()),
                )
            }
        }
    }

    pub async fn rebooting(
        &self,
        ota_request: OtaRequest,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
    ) -> OtaStatus {
        if ota_status_publisher
            .send(OtaStatus::Rebooting(ota_request.clone()))
            .await
            .is_err()
        {
            warn!("ota_status_publisher dropped before send rebooting_status")
        };

        info!("Rebooting in 5 seconds");

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        #[cfg(not(test))]
        if let Err(error) = crate::power_management::reboot() {
            let message = "Unable to run reboot command";
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        OtaStatus::Rebooted
    }

    pub async fn success(&self) -> OtaStatus {
        if self.state_repository.exists() {
            info!("Found pending update");

            let ota_state = self.state_repository.read();
            if ota_state.is_err() {
                let message = "Unable to read pending ota state".to_string();
                error!("{message} : {}", ota_state.unwrap_err());
                return OtaStatus::Failure(OtaError::IO(message), None);
            }

            let ota_state = ota_state.unwrap();
            let request_uuid = ota_state.uuid;
            let ota_request = OtaRequest {
                uuid: request_uuid,
                url: "".to_string(),
            };

            let ota_result = if let Err(error) = self.do_pending_ota(&ota_state).await {
                OtaStatus::Failure(error, Some(ota_request.clone()))
            } else {
                OtaStatus::Success(ota_request)
            };

            let _ = self.state_repository.clear().map_err(|error| {
                warn!("Error during clear of state repository-> {:?}", error);
            });

            ota_result
        } else {
            OtaStatus::NoPendingOta
        }
    }

    pub async fn do_pending_ota(&self, state: &PersistentState) -> Result<(), OtaError> {
        const GOOD_STATE: &str = "good";

        let booted_slot = self.system_update.boot_slot().await.map_err(|error| {
            let message = "Unable to identify the booted slot";
            error!("{message}: {error}");
            OtaError::Internal(message)
        })?;

        if state.slot == booted_slot {
            let message = "Unable to switch slot";
            return Err(OtaError::SystemRollback(message));
        }

        let primary_slot = self.system_update.get_primary().await.map_err(|error| {
            let message = "Unable to get the current primary slot";
            error!("{message}: {error}");
            OtaError::Internal(message)
        })?;

        let (marked_slot, _) = self
            .system_update
            .mark(GOOD_STATE, &primary_slot)
            .await
            .map_err(|error| {
                let message = "Unable to run marking slot operation";
                error!("{message}: {error}");
                OtaError::Internal(message)
            })?;

        if primary_slot != marked_slot {
            let message = "Unable to mark slot";
            Err(OtaError::Internal(message))
        } else {
            Ok(())
        }
    }

    pub async fn handle_ota_event(
        &self,
        ota_status: OtaStatus,
        ota_status_publisher: &mpsc::Sender<OtaStatus>,
        data: HashMap<String, AstarteType>,
    ) -> OtaStatus {
        let mut ota_status = ota_status.clone();

        loop {
            ota_status = match ota_status {
                OtaStatus::Idle => OtaStatus::Init,
                OtaStatus::Init => self.acknowledged(ota_status_publisher, data.clone()).await,
                OtaStatus::Acknowledged(ota_request) => {
                    self.downloading(ota_request, ota_status_publisher).await
                }
                OtaStatus::Downloading(ota_request, _) => {
                    self.deploying(ota_request, ota_status_publisher).await
                }
                OtaStatus::Deploying(ota_request) => {
                    self.deployed(ota_request, ota_status_publisher).await
                }
                OtaStatus::Deployed(ota_request) => {
                    self.rebooting(ota_request, ota_status_publisher).await
                }
                OtaStatus::Rebooted => self.success().await,
                OtaStatus::Error(ota_error, ota_request) => {
                    OtaStatus::Failure(ota_error, Some(ota_request))
                }
                OtaStatus::Rebooting(_)
                | OtaStatus::NoPendingOta
                | OtaStatus::Success(_)
                | OtaStatus::Failure(_, _) => break,
            };

            *self.ota_status.write().await = ota_status.clone();
        }

        if let Some(path) = self.get_update_file_path().to_str() {
            if let Err(e) = std::fs::remove_file(path) {
                error!("Unable to remove {}: {}", path, e);
            }
        }

        *self.ota_status.write().await = OtaStatus::Idle;
        ota_status
    }
}

pub async fn wget(
    url: &str,
    file_path: &str,
    request_uuid: &Uuid,
    ota_status_publisher: &mpsc::Sender<OtaStatus>,
) -> Result<(), OtaError> {
    use tokio_stream::StreamExt;

    if std::path::Path::new(file_path).exists() {
        std::fs::remove_file(file_path)
            .unwrap_or_else(|e| panic!("Unable to remove {}: {}", file_path, e));
    }
    info!("Downloading {:?}", url);

    let result_response = reqwest::get(url).await;

    match result_response {
        Err(err) => {
            let message = "Error downloading update".to_string();
            error!("{message}: {err:?}");
            Err(OtaError::Network(message))
        }
        Ok(response) => {
            debug!("Writing {file_path}");

            let total_size = response
                .content_length()
                .and_then(|size| if size == 0 { None } else { Some(size) })
                .ok_or_else(|| {
                    OtaError::Network(format!("Unable to get content length from: {url}"))
                })?;

            let mut downloaded: u64 = 0;
            let mut stream = response.bytes_stream();

            let mut os_file = std::fs::File::create(&file_path).map_err(|error| {
                let message = format!("Unable to create ota_file in {file_path:?}");
                error!("{message} : {error:?}");
                OtaError::IO(message)
            })?;

            while let Some(chunk_result) = stream.next().await {
                let chunk = chunk_result.map_err(|error| {
                    let message = "Unable to parse response".to_string();
                    error!("{message} : {error:?}");
                    OtaError::Network(message)
                })?;

                let mut content = std::io::Cursor::new(&chunk);

                std::io::copy(&mut content, &mut os_file).map_err(|error| {
                    let message = format!("Unable to write chunk to ota_file in {file_path:?}");
                    error!("{message} : {error:?}");
                    OtaError::IO(message)
                })?;

                downloaded = std::cmp::min(downloaded + (chunk.len() as u64), total_size);
                let progress = ((downloaded / total_size) * 100) as i32;

                if ota_status_publisher
                    .send(OtaStatus::Downloading(
                        OtaRequest {
                            uuid: *request_uuid,
                            url: "".to_string(),
                        },
                        progress,
                    ))
                    .await
                    .is_err()
                {
                    warn!("ota_status_publisher dropped before send downloading_status")
                }
            }

            if total_size != downloaded {
                let message = "Unable to download file".to_string();
                error!("{message}");
                Err(OtaError::Network(message))
            } else {
                Ok(())
            }
        }
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
            OtaStatus::Deploying(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Deploying".to_string();
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
            OtaError::Cancelled => ota_status_message.status_code = "Cancelled".to_string(),
        }

        ota_status_message
    }
}

async fn send_ota_event(sdk: &impl Publisher, ota_status: &OtaStatus) -> Result<(), OtaError> {
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

    sdk.send_object(
        "io.edgehog.devicemanager.OTAResponse",
        "/response",
        ota_event,
    )
    .await
    .map_err(|error| {
        let message = "Unable to publish ota_event".to_string();
        error!("{message} : {error}");
        OtaError::Network(message)
    })?;

    Ok(())
}
