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
use std::path::PathBuf;
use std::sync::Arc;

use astarte_device_sdk::types::AstarteType;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::DeviceManagerError;
use crate::ota::rauc::OTARauc;
use crate::ota::{OtaError, SystemUpdate};
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;

#[derive(Serialize, Deserialize, Debug)]
pub struct PersistentState {
    pub uuid: Uuid,
    pub slot: String,
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

#[derive(PartialEq, Clone, Debug)]
pub struct OtaRequest {
    pub uuid: Uuid,
    pub url: String,
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

impl OtaStatus {
    pub fn ota_request(&self) -> Option<&OtaRequest> {
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

/// Provides ota resource accessibility only by talking with it.
pub struct Ota<'a> {
    pub system_update: Box<dyn SystemUpdate + 'a>,
    pub state_repository: Box<dyn StateRepository<PersistentState> + 'a>,
    pub download_file_path: String,
    pub ota_status: Arc<RwLock<OtaStatus>>,
}

impl<'a> Ota<'a> {
    pub async fn new(opts: &crate::DeviceManagerOptions) -> Result<Ota<'a>, DeviceManagerError> {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use astarte_device_sdk::types::AstarteType;
    use httpmock::prelude::*;
    use tempdir::TempDir;
    use tokio::sync::{mpsc, RwLock};
    use uuid::Uuid;

    use crate::error::DeviceManagerError;
    use crate::ota::ota_handle::{wget, Ota, OtaRequest, OtaStatus, PersistentState};
    use crate::ota::rauc::BundleInfo;
    use crate::ota::{MockSystemUpdate, OtaError};
    use crate::repository::MockStateRepository;

    /// Creates a temporary directory that will be deleted when the returned TempDir is dropped.
    fn temp_dir() -> (TempDir, String) {
        let dir = TempDir::new("edgehog").unwrap();
        let str = dir.path().to_str().unwrap().to_string();

        (dir, str)
    }

    impl<'a> Ota<'a> {
        pub fn mock_new(
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
