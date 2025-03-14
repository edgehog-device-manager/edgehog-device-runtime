// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;
use std::fmt::Display;

use async_trait::async_trait;
use event::OtaRequest;
use futures::stream::BoxStream;
use futures::TryStreamExt;
#[cfg(all(feature = "zbus", target_os = "linux"))]
use ota_handler::{OtaEvent, OtaInProgress, OtaMessage, OtaStatusMessage};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::controller::actor::Actor;
use crate::error::DeviceManagerError;
use crate::repository::StateRepository;

pub mod event;
#[cfg(all(feature = "zbus", target_os = "linux"))]
pub(crate) mod ota_handler;
#[cfg(test)]
mod ota_handler_test;
pub(crate) mod rauc;

/// Provides deploying progress information.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeployProgress {
    percentage: i32,
    message: String,
}

impl Display for DeployProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "progress {}%: {}", self.percentage, self.message)
    }
}

/// Provides the status of the deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployStatus {
    Progress(DeployProgress),
    Completed { signal: i32 },
}

/// Stream of the [`DeployStatus`] events
pub type ProgressStream = BoxStream<'static, Result<DeployStatus, DeviceManagerError>>;

/// A **trait** required for all SystemUpdate handlers that want to update a system.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SystemUpdate: Send + Sync {
    /// Install the bundle from the provided source.
    ///
    /// The source can either be a path to a file or an URL.
    async fn install_bundle(&self, source: &str) -> Result<(), DeviceManagerError>;
    async fn last_error(&self) -> Result<String, DeviceManagerError>;
    async fn operation(&self) -> Result<String, DeviceManagerError>;
    async fn boot_slot(&self) -> Result<String, DeviceManagerError>;
    async fn receive_completed(&self) -> Result<ProgressStream, DeviceManagerError>;
    async fn get_primary(&self) -> Result<String, DeviceManagerError>;
    async fn mark(
        &self,
        state: &str,
        slot_identifier: &str,
    ) -> Result<(String, String), DeviceManagerError>;
}

/// Edgehog OTA error.
///
/// Possible errors returned by OTA.
#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum OtaError {
    /// Invalid OTA update request received
    #[error("InvalidRequestError: {0}")]
    Request(&'static str),
    #[error("UpdateAlreadyInProgress")]
    /// Attempted to perform OTA operation while there is another one already active*/
    UpdateAlreadyInProgress,
    #[error("NetworkError: {0}")]
    /// A generic network error occurred
    Network(String),
    #[error("IOError: {0}")]
    /// A filesystem error occurred
    Io(String),
    #[error("InternalError: {0}")]
    /// An Internal error occurred during OTA procedure
    Internal(&'static str),
    #[error("InvalidBaseImage: {0}")]
    /// Invalid OTA image received
    InvalidBaseImage(String),
    #[error("SystemRollback: {0}")]
    /// The OTA procedure boot on the wrong partition
    SystemRollback(&'static str),
    /// OTA update aborted by Edgehog half way during the procedure
    #[error("Canceled")]
    Canceled,
    /// Attempted to start OTA operation while ota status is different from Idle
    #[error("Inconsistent ota state")]
    InconsistentState,
}

impl Default for DeployStatus {
    fn default() -> Self {
        DeployStatus::Progress(DeployProgress::default())
    }
}

const DEPLOY_PERC_ROUNDING_STEP: i32 = 10;

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistentState {
    pub uuid: Uuid,
    pub slot: String,
}

#[derive(Clone, PartialEq, Debug)]
pub enum OtaStatus {
    /// The device is waiting an OTA event
    Idle,
    /// The device initializing the OTA procedure
    Init(OtaId),
    /// The device didn't has an OTA procedure pending
    NoPendingOta,
    /// The device received a valid OTA Request
    Acknowledged(OtaId),
    /// The device is in the process of deploying the update
    Deploying(OtaId, DeployProgress),
    /// The device deployed the update
    Deployed(OtaId),
    /// The device is in the process of rebooting
    Rebooting(OtaId),
    /// The device was rebooted
    Rebooted,
    /// The update procedure succeeded.
    Success(OtaId),
    /// The update procedure failed.
    Failure(OtaError, Option<OtaId>),
}

impl OtaStatus {
    // Checks if the OTA is cancellable
    fn is_cancellable(&self) -> bool {
        match self {
            OtaStatus::Init(_) | OtaStatus::Acknowledged(_) => true,
            OtaStatus::Idle
            | OtaStatus::NoPendingOta
            | OtaStatus::Deploying(_, _)
            | OtaStatus::Deployed(_)
            | OtaStatus::Rebooting(_)
            | OtaStatus::Rebooted
            | OtaStatus::Success(_)
            | OtaStatus::Failure(_, _) => false,
        }
    }

    fn ota_id(&self) -> Option<OtaId> {
        match self {
            OtaStatus::Idle | OtaStatus::NoPendingOta | OtaStatus::Rebooted => None,
            OtaStatus::Init(id)
            | OtaStatus::Acknowledged(id)
            | OtaStatus::Deploying(id, _)
            | OtaStatus::Deployed(id)
            | OtaStatus::Rebooting(id)
            | OtaStatus::Success(id) => Some(id.clone()),
            OtaStatus::Failure(_, id) => id.clone(),
        }
    }

    /// Converts the status into an event
    fn as_event(&self) -> Option<OtaEvent> {
        let mut ota_event = OtaEvent {
            requestUUID: "".to_string(),
            status: "".to_string(),
            statusProgress: 0,
            statusCode: "".to_string(),
            message: "".to_string(),
        };

        match self {
            OtaStatus::Acknowledged(ota_request) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Acknowledged".to_string();
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
            OtaStatus::Failure(ota_error, Some(ota_request)) => {
                ota_event.requestUUID = ota_request.uuid.to_string();
                ota_event.status = "Failure".to_string();
                let ota_status_message = OtaStatusMessage::from(ota_error);
                ota_event.statusCode = ota_status_message.status_code;
                ota_event.message = ota_status_message.message;
            }
            OtaStatus::Idle
            | OtaStatus::Init(_)
            | OtaStatus::NoPendingOta
            | OtaStatus::Rebooted
            | OtaStatus::Failure(_, None) => return None,
        }

        if ota_event.requestUUID.is_empty() {
            error!("Unable to convert ota_event: request_uuid is empty");
            None
        } else {
            Some(ota_event)
        }
    }
}

impl Display for OtaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OtaStatus::Idle => write!(f, "Idle"),
            OtaStatus::Init(req) => write!(f, "Init {req}"),
            OtaStatus::NoPendingOta => write!(f, "NoPendingOta"),
            OtaStatus::Acknowledged(req) => write!(f, "Acknowledged {req}"),
            OtaStatus::Deploying(req, progress) => write!(f, "Deploying {req} {progress}"),
            OtaStatus::Deployed(req) => write!(f, "Deployed {req}"),
            OtaStatus::Rebooting(req) => write!(f, "Rebooting {req}"),
            OtaStatus::Rebooted => write!(f, "Rebooted"),
            OtaStatus::Success(req) => write!(f, "Success {req}"),
            OtaStatus::Failure(err, req) => {
                write!(f, "Failure")?;

                if let Some(req) = req {
                    write!(f, " {req}")?;
                }

                write!(f, ": {err}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtaId {
    pub uuid: Uuid,
    pub url: String,
}

impl From<OtaRequest> for OtaId {
    fn from(value: OtaRequest) -> Self {
        Self {
            uuid: value.uuid.0,
            url: value.url,
        }
    }
}

impl Display for OtaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.uuid)
    }
}

/// Provides OTA resource accessibility only by talking with it.
pub struct Ota<T, U>
where
    T: SystemUpdate,
    U: StateRepository<PersistentState>,
{
    pub system_update: T,
    pub state_repository: U,
    pub ota_status: OtaStatus,
    pub flag: OtaInProgress,
    pub publisher_tx: mpsc::Sender<OtaStatus>,
}

#[async_trait]
impl<T, U> Actor for Ota<T, U>
where
    T: SystemUpdate,
    U: StateRepository<PersistentState>,
{
    type Msg = OtaMessage;

    fn task() -> &'static str {
        "ota"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        if self.state_repository.exists().await {
            self.ota_status = OtaStatus::Rebooted;
        }

        // Not cancellable after a reboot, this is just a placeholder
        let cancel = CancellationToken::new();

        // Always run the publish and cleanup
        self.handle_ota_update(cancel).await;

        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        if self.ota_status != OtaStatus::Idle {
            error!("ota request already in progress");

            return Err(OtaError::InconsistentState.into());
        }

        self.ota_status = OtaStatus::Init(msg.ota_id);
        self.handle_ota_update(msg.cancel).await;

        Ok(())
    }
}

impl<T, U> Ota<T, U>
where
    T: SystemUpdate,
    U: StateRepository<PersistentState>,
{
    pub fn new(
        tx_publisher: mpsc::Sender<OtaStatus>,
        flag: OtaInProgress,
        system_update: T,
        state_repository: U,
    ) -> Result<Self, DeviceManagerError> {
        Ok(Ota {
            system_update,
            state_repository,
            ota_status: OtaStatus::Idle,
            flag,
            publisher_tx: tx_publisher,
        })
    }

    pub async fn last_error(&self) -> Result<String, DeviceManagerError> {
        self.system_update.last_error().await
    }

    /// Handle the transition to the deploying status.
    pub async fn bootslot(&self, ota_request: OtaId) -> OtaStatus {
        let booted_slot = match self.system_update.boot_slot().await {
            Ok(slot) => slot,
            Err(err) => {
                let message = "Unable to identify the booted slot";

                error!("{message}: {err}");

                return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
            }
        };

        let state = PersistentState {
            uuid: ota_request.uuid,
            slot: booted_slot,
        };

        if let Err(error) = self.state_repository.write(&state).await {
            let message = "Unable to persist ota state".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Io(message), Some(ota_request));
        };

        OtaStatus::Deploying(ota_request.clone(), DeployProgress::default())
    }

    /// Handle the transition to the deployed status.
    pub async fn deploy(&self, ota_request: OtaId) -> OtaStatus {
        let res = self.system_update.install_bundle(&ota_request.url).await;
        if let Err(error) = res {
            let message = "Unable to install ota image".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::InvalidBaseImage(message), Some(ota_request));
        }

        if let Err(error) = self.system_update.operation().await {
            let message = "Unable to get status of ota operation";
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request));
        }

        let stream = self.system_update.receive_completed().await;
        let stream = match stream {
            Ok(stream) => stream,
            Err(err) => {
                let message = "Unable to get status of ota operation";
                error!("{message} : {err}");
                return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request));
            }
        };

        let signal = stream
            .try_fold(
                DeployStatus::Progress(DeployProgress::default()),
                |prev_status, status| {
                    let ota_request_cl = ota_request.clone();
                    let status_cl = status.clone();

                    async move {
                        let progress = match status {
                            DeployStatus::Completed { .. } => {
                                return Ok(status);
                            }
                            DeployStatus::Progress(progress) => progress,
                        };

                        let last_progress_sent = match &prev_status {
                            DeployStatus::Progress(last_progress) => last_progress.percentage,
                            _ => progress.percentage,
                        };

                        if (progress.percentage - last_progress_sent) >= DEPLOY_PERC_ROUNDING_STEP {
                            let res = self
                                .publisher_tx
                                .send(OtaStatus::Deploying(ota_request_cl, progress))
                                .await;

                            if let Err(err) = res {
                                error!("couldn't send progress update: {err}")
                            }
                            return Ok(status_cl);
                        }
                        Ok(prev_status)
                    }
                },
            )
            .await;

        let signal = match signal {
            Ok(DeployStatus::Completed { signal }) => signal,
            Ok(DeployStatus::Progress(_)) => {
                let message = "No progress completion event received";
                error!("{message}");
                return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request));
            }
            Err(err) => {
                let message = "Unable to receive the install completed event";
                error!("{message} : {err}");
                return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request));
            }
        };

        info!("Completed signal! {:?}", signal);

        match signal {
            0 => {
                info!("Update successful");

                OtaStatus::Deployed(ota_request)
            }
            _ => {
                let message = format!("Update failed with signal {signal}");

                match self.last_error().await {
                    Ok(err) => {
                        error!("{message}: {err}");
                    }
                    Err(err) => {
                        error!("{message}: {}", stable_eyre::Report::new(err));
                    }
                }

                OtaStatus::Failure(OtaError::InvalidBaseImage(message), Some(ota_request))
            }
        }
    }

    /// Handle the transition to rebooting status.
    pub async fn reboot(&self, ota_request: OtaId) -> OtaStatus {
        if self
            .publisher_tx
            .send(OtaStatus::Rebooting(ota_request.clone()))
            .await
            .is_err()
        {
            warn!("ota_status_publisher dropped before sending rebooting_status")
        };

        info!("Rebooting the device");

        #[cfg(not(test))]
        if let Err(error) = crate::power_management::reboot().await {
            let message = "Unable to run reboot command";
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        OtaStatus::Rebooted
    }

    /// Handle the transition to success status.
    pub async fn check_reboot(&self) -> OtaStatus {
        if !self.state_repository.exists().await {
            return OtaStatus::NoPendingOta;
        }

        info!("Found pending update");

        let ota_state = match self.state_repository.read().await {
            Ok(state) => state,
            Err(err) => {
                let message = "Unable to read pending ota state".to_string();
                error!("{message} : {}", err);
                return OtaStatus::Failure(OtaError::Io(message), None);
            }
        };

        let request_uuid = ota_state.uuid;
        let ota_request = OtaId {
            uuid: request_uuid,
            url: "".to_string(),
        };

        if let Err(error) = self.do_pending_ota(&ota_state).await {
            return OtaStatus::Failure(error, Some(ota_request));
        }

        OtaStatus::Success(ota_request)
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
            error!("{message}");
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

    pub async fn next(&mut self) {
        self.ota_status = match self.ota_status.clone() {
            OtaStatus::Init(req) => OtaStatus::Acknowledged(req),
            OtaStatus::Acknowledged(ota_request) => self.bootslot(ota_request).await,
            OtaStatus::Deploying(ota_request, _) => self.deploy(ota_request).await,
            OtaStatus::Deployed(ota_request) => self.reboot(ota_request).await,
            OtaStatus::Rebooted => self.check_reboot().await,
            rebooting @ OtaStatus::Rebooting(_) => rebooting,
            OtaStatus::Idle
            | OtaStatus::NoPendingOta
            | OtaStatus::Success(_)
            | OtaStatus::Failure(_, _) => OtaStatus::Idle,
        };
    }

    pub async fn handle_ota_update(&mut self, cancel: CancellationToken) {
        let mut check_cancel = true;

        while self.is_ota_in_progress() {
            self.publish_status(self.ota_status.clone()).await;

            if self.ota_status.is_cancellable() {
                if cancel.run_until_cancelled(self.next()).await.is_none() {
                    check_cancel = false;
                    info!("OTA update cancelled");

                    self.ota_status =
                        OtaStatus::Failure(OtaError::Canceled, self.ota_status.ota_id())
                }

                continue;
            } else if check_cancel && cancel.is_cancelled() {
                // Not cancellable
                check_cancel = false;

                self.publish_status(OtaStatus::Failure(OtaError::Canceled, None))
                    .await;
            }

            self.next().await;
        }

        // Publish the final status
        self.publish_status(self.ota_status.clone()).await;

        self.clear().await;
    }

    async fn clear(&mut self) {
        if self.state_repository.exists().await {
            let _ = self.state_repository.clear().await.map_err(|error| {
                error!("Error clearing the state repository: {:?}", error);
            });
        }

        self.ota_status = OtaStatus::Idle;
    }

    async fn publish_status(&self, status: OtaStatus) {
        if self.publisher_tx.send(status).await.is_err() {
            error!(
                "ota publisher disconnected before sending status {}",
                self.ota_status
            )
        }
    }

    /// Check if the OTA status is in progress
    fn is_ota_in_progress(&self) -> bool {
        // The OTA is always considered in progress until the Idle state
        let in_progress = self.ota_status != OtaStatus::Idle;

        self.flag.set_in_progress(in_progress);

        in_progress
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::PathBuf;
    use std::time::Duration;

    use futures::StreamExt;
    use mockall::Sequence;
    use tempdir::TempDir;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    use crate::error::DeviceManagerError;
    use crate::ota::ota_handler_test::deploy_status_stream;
    use crate::ota::{DeployProgress, DeployStatus, MockSystemUpdate, OtaError, SystemUpdate};
    use crate::ota::{Ota, OtaId, OtaStatus, PersistentState};
    use crate::repository::file_state_repository::FileStateError;
    use crate::repository::{MockStateRepository, StateRepository};

    use super::ota_handler::OtaInProgress;

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir(prefix: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new(&format!("edgehog-{prefix}")).unwrap();
        let path = dir.path().to_owned();

        (dir, path)
    }

    impl<T, U> Ota<T, U>
    where
        T: SystemUpdate,
        U: StateRepository<PersistentState>,
    {
        /// Create the mock with a non existent download path
        pub fn mock_new(
            system_update: T,
            state_repository: U,
            publisher_tx: mpsc::Sender<OtaStatus>,
        ) -> Self {
            Ota {
                system_update,
                state_repository,
                ota_status: OtaStatus::Idle,
                publisher_tx,
                flag: OtaInProgress::default(),
            }
        }

        /// Create the mock with a usable download path
        pub fn mock_new_with_path(
            system_update: T,
            state_repository: U,
            prefix: &str,
            publisher_tx: mpsc::Sender<OtaStatus>,
        ) -> (Self, TempDir) {
            let (dir, _) = temp_dir(prefix);
            let mock = Ota {
                system_update,
                state_repository,
                ota_status: OtaStatus::Idle,
                publisher_tx,
                flag: OtaInProgress::default(),
            };

            (mock, dir)
        }
    }

    #[tokio::test]
    async fn last_error_ok() {
        let mut system_update = MockSystemUpdate::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        system_update
            .expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let (tx, _rx) = mpsc::channel(10);

        let ota = Ota::mock_new(system_update, state_mock, tx);

        let last_error_result = ota.last_error().await;

        assert!(last_error_result.is_ok());
        assert_eq!("Unable to deploy image", last_error_result.unwrap());
    }

    #[tokio::test]
    async fn last_error_fail() {
        let mut system_update = MockSystemUpdate::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        system_update.expect_last_error().returning(|| {
            Err(DeviceManagerError::Fatal(
                "Unable to call last error".to_string(),
            ))
        });

        let (tx, _rx) = mpsc::channel(10);

        let ota = Ota::mock_new(system_update, state_mock, tx);

        let last_error_result = ota.last_error().await;

        assert!(last_error_result.is_err());
        assert!(matches!(
            last_error_result.err().unwrap(),
            DeviceManagerError::Fatal(_)
        ))
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_request() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();
        let mut seq = Sequence::new();

        system_update
            .expect_boot_slot()
            .once()
            .in_sequence(&mut seq)
            .returning(|| Err(DeviceManagerError::Fatal("boot slot".to_string())));

        let ota_request = OtaId {
            uuid: Uuid::new_v4(),
            url: "https://example.com/ota.bin".to_string(),
        };

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "fail_ota_request",
            ota_status_publisher,
        );

        let ota_status = ota.bootslot(ota_request).await;

        let receive_result = ota_status_receiver.try_recv();
        assert!(receive_result.is_err());

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_info() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();
        let mut seq = Sequence::new();

        system_update
            .expect_boot_slot()
            .once()
            .in_sequence(&mut seq)
            .returning(|| Err(DeviceManagerError::Fatal("boot slot".to_string())));

        let ota_request = OtaId {
            uuid: Uuid::new_v4(),
            url: "https://example.com/ota.bin".to_string(),
        };

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "fail_ota_info",
            ota_status_publisher,
        );

        let ota_status = ota.bootslot(ota_request).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_write_state() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        state_mock.expect_write().returning(|_| {
            Err(FileStateError::Write {
                path: "/ota.bin".into(),
                backtrace: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
            })
        });

        system_update
            .expect_boot_slot()
            .returning(|| Ok("A".to_string()));

        let ota_url = "https://examples.com/ota.bin".to_string();

        let (publisher_tx, publisher_rx) = mpsc::channel(10);
        let (ota, _dir) =
            Ota::mock_new_with_path(system_update, state_mock, "fail_write_state", publisher_tx);

        let ota_id = OtaId {
            uuid: Uuid::new_v4(),
            url: ota_url,
        };
        let ota_status = ota.bootslot(ota_id.clone()).await;

        assert!(publisher_rx.is_empty());

        assert_eq!(
            ota_status,
            OtaStatus::Failure(
                OtaError::Io("Unable to persist ota state".to_string()),
                Some(ota_id),
            ),
        );
    }

    #[tokio::test]
    async fn try_to_deployed_fail_install_bundle() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut system_update = MockSystemUpdate::new();

        system_update
            .expect_install_bundle()
            .returning(|_| Err(DeviceManagerError::Fatal("install fail".to_string())));

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "fail_install_bundle",
            ota_status_publisher,
        );

        let ota_status = ota.deploy(OtaId::default()).await;

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
        system_update
            .expect_operation()
            .returning(|| Err(DeviceManagerError::Fatal("operation call fail".to_string())));

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "deployed_fail_operation",
            ota_status_publisher,
        );

        let ota_status = ota.deploy(OtaId::default()).await;

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
            Err(DeviceManagerError::Fatal(
                "receive_completed call fail".to_string(),
            ))
        });

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "deployed_fail_receive_completed",
            ota_status_publisher,
        );

        let ota_status = ota.deploy(OtaId::default()).await;

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
            .returning(|| deploy_status_stream([DeployStatus::Completed { signal: -1 }]));
        system_update
            .expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let (ota_status_publisher, _) = mpsc::channel(1);
        let (ota, _dir) = Ota::mock_new_with_path(
            system_update,
            state_mock,
            "deployed_fail_signal",
            ota_status_publisher,
        );

        let ota_status = ota.deploy(OtaId::default()).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut system_update = MockSystemUpdate::new();
        let mut seq = Sequence::new();

        system_update
            .expect_install_bundle()
            .once()
            .in_sequence(&mut seq)
            .returning(|_| Ok(()));
        system_update
            .expect_operation()
            .once()
            .in_sequence(&mut seq)
            .returning(|| Ok("".to_string()));
        system_update
            .expect_receive_completed()
            .once()
            .in_sequence(&mut seq)
            .returning(|| {
                let progress = [
                    DeployStatus::Progress(DeployProgress {
                        percentage: 50,
                        message: "Copy image".to_string(),
                    }),
                    DeployStatus::Progress(DeployProgress {
                        percentage: 100,
                        message: "Installing is done".to_string(),
                    }),
                    DeployStatus::Completed { signal: 0 },
                ]
                .map(Ok);

                Ok(futures::stream::iter(progress).boxed())
            });

        let (publisher_tx, mut publisher_rx) = mpsc::channel(3);
        let (ota, _dir) =
            Ota::mock_new_with_path(system_update, state_mock, "deployed_success", publisher_tx);

        let ota_id = OtaId {
            uuid: Uuid::new_v4(),
            url: String::new(),
        };
        let status = ota.deploy(ota_id.clone()).await;

        let exp = [
            OtaStatus::Deploying(
                ota_id.clone(),
                DeployProgress {
                    percentage: 50,
                    message: "Copy image".to_string(),
                },
            ),
            OtaStatus::Deploying(
                ota_id.clone(),
                DeployProgress {
                    percentage: 100,
                    message: "Installing is done".to_string(),
                },
            ),
        ];

        for status in exp {
            let val = tokio::time::timeout(Duration::from_secs(2), publisher_rx.recv())
                .await
                .unwrap()
                .unwrap();

            assert_eq!(val, status);
        }

        assert_eq!(status, OtaStatus::Deployed(ota_id));
    }

    #[tokio::test]
    async fn try_to_rebooting_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();
        let ota_request = OtaId::default();

        let (ota_status_publisher, mut ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let ota_status = ota.reboot(ota_request).await;

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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let ota_status = ota.check_reboot().await;

        assert!(matches!(ota_status, OtaStatus::NoPendingOta));
    }

    #[tokio::test]
    async fn try_to_success_fail_read_state() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let system_update = MockSystemUpdate::new();

        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Err(FileStateError::Write {
                path: "/ota.bin".into(),
                backtrace: io::Error::new(io::ErrorKind::PermissionDenied, "permission denied"),
            })
        });

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let ota_status = ota.check_reboot().await;

        assert!(matches!(ota_status, OtaStatus::Failure(OtaError::Io(_), _)));
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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let ota_status = ota.check_reboot().await;

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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let ota_status = ota.check_reboot().await;

        assert!(matches!(ota_status, OtaStatus::Success(_)));
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
            Err(DeviceManagerError::Fatal(
                "unable to call boot slot".to_string(),
            ))
        });

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);

        let state = ota.state_repository.read().await.unwrap();
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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);

        let state = ota.state_repository.read().await.unwrap();
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
            Err(DeviceManagerError::Fatal(
                "unable to call boot slot".to_string(),
            ))
        });

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);
        let state = ota.state_repository.read().await.unwrap();
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
            Err(DeviceManagerError::Fatal(
                "Unable to call mark function".to_string(),
            ))
        });

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);

        let state = ota.state_repository.read().await.unwrap();
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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);

        let state = ota.state_repository.read().await.unwrap();
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

        let (ota_status_publisher, _ota_status_receiver) = mpsc::channel(1);
        let ota = Ota::mock_new(system_update, state_mock, ota_status_publisher);

        let state = ota.state_repository.read().await.unwrap();
        let result = ota.do_pending_ota(&state).await;
        assert!(result.is_ok());
    }
}
