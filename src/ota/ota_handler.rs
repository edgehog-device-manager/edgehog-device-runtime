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
use std::path::PathBuf;

use astarte_device_sdk::types::AstarteType;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::data::Publisher;
use crate::error::DeviceManagerError;
use crate::ota::rauc::OTARauc;
use crate::ota::Ota;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;

#[derive(Serialize, Deserialize, Debug)]
struct PersistentState {
    uuid: Uuid,
    slot: String,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq)]
pub enum OtaError {
    #[error("InvalidRequestError: {0}")]
    Request(String),
    #[error("UpdateAlreadyInProgress")]
    UpdateAlreadyInProgress,
    #[error("NetworkError: {0}")]
    Network(String),
    #[error("IOError: {0}")]
    IO(String),
    #[error("InternalError: {0}")]
    Internal(String),
    #[error("InvalidBaseImage: {0}")]
    InvalidBaseImage(String),
    #[error("SystemRollback: {0}")]
    SystemRollback(String),
    #[error("Cancelled")]
    Cancelled,
}

#[derive(Clone, PartialEq)]
enum OtaStatus {
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
struct OtaEvent {
    requestUUID: String,
    status: String,
    statusProgress: i32,
    statusCode: String,
    message: String,
}

struct OtaStatusMessage {
    status_code: String,
    message: String,
}

#[derive(PartialEq, Clone)]
pub struct OtaRequest {
    uuid: Uuid,
    url: String,
}

pub struct OtaHandler<'a> {
    ota: Box<dyn Ota + 'a>,
    state_repository: Box<dyn StateRepository<PersistentState> + 'a>,
    download_file_path: String,
    ota_status: OtaStatus,
}

impl<'a> OtaHandler<'a> {
    pub async fn new(
        opts: &crate::DeviceManagerOptions,
    ) -> Result<OtaHandler<'a>, DeviceManagerError> {
        let ota = OTARauc::new().await?;

        Ok(OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(FileStateRepository::new(
                opts.store_directory.clone(),
                "state.json".to_owned(),
            )),
            download_file_path: opts.download_directory.clone(),
            ota_status: OtaStatus::Rebooted,
        })
    }

    pub async fn last_error(&self) -> Result<String, DeviceManagerError> {
        self.ota.last_error().await
    }

    fn get_update_file_path(&self) -> PathBuf {
        std::path::Path::new(&self.download_file_path).join("update.bin")
    }

    async fn acknowledged(
        &self,
        publisher: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> OtaStatus {
        if !data.contains_key("url") || !data.contains_key("uuid") {
            return OtaStatus::Failure(
                OtaError::Request("Unable to find data in the OTA request".to_owned()),
                None,
            );
        }

        if let (AstarteType::String(request_url), AstarteType::String(request_uuid_str)) =
            (&data["url"], &data["uuid"])
        {
            let request_uuid = Uuid::parse_str(request_uuid_str);
            if request_uuid.is_err() {
                return OtaStatus::Failure(
                    OtaError::Request("Unable to parse request_uuid".to_owned()),
                    None,
                );
            }

            let request_uuid = request_uuid.unwrap();

            let ack_status = OtaStatus::Acknowledged(OtaRequest {
                uuid: request_uuid,
                url: request_url.to_string(),
            });

            send_ota_event(publisher, &ack_status).await;
            ack_status
        } else {
            let message = "Got invalid data in OTARequest".to_string();
            error!("{message}: {:?}", data);
            OtaStatus::Failure(OtaError::Request(message), None)
        }
    }

    async fn downloading(&self, ota_request: OtaRequest, publisher: &impl Publisher) -> OtaStatus {
        let downloading_status = OtaStatus::Downloading(ota_request, 0);
        send_ota_event(publisher, &downloading_status).await;
        downloading_status
    }

    async fn deploying(&self, ota_request: OtaRequest, publisher: &impl Publisher) -> OtaStatus {
        let download_file_path = self.get_update_file_path();

        let download_file_path = download_file_path.to_str();
        if download_file_path.is_none() {
            return OtaStatus::Failure(
                OtaError::IO("Wrong download file path".to_string()),
                Some(ota_request),
            );
        }

        let download_file_path = download_file_path.unwrap();

        let mut ota_download_result = wget(
            &ota_request.url,
            download_file_path,
            &ota_request.uuid,
            publisher,
        )
        .await;
        for i in 1..5 {
            if let Err(error) = ota_download_result {
                let wait = u64::pow(2, i);
                let message = "Error downloading update".to_string();
                error!("{message}: {:?}", error);
                error!("Next attempt in {}s", wait);

                send_ota_event(publisher, &OtaStatus::Error(error, ota_request.clone())).await;

                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                ota_download_result = wget(
                    &ota_request.url,
                    download_file_path,
                    &ota_request.uuid,
                    publisher,
                )
                .await;
            } else {
                break;
            }
        }

        if let Err(error) = ota_download_result {
            OtaStatus::Failure(error, Some(ota_request.clone()))
        } else {
            let bundle_info = self.ota.info(download_file_path).await;
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

            let system_image_info = self.ota.compatible().await;
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

            let booted_slot = self.ota.boot_slot().await;
            if booted_slot.is_err() {
                let message = "Unable to identify the booted slot".to_string();
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
            send_ota_event(publisher, &deploying_state).await;

            deploying_state
        }
    }

    async fn deployed(&self, ota_request: OtaRequest, publisher: &impl Publisher) -> OtaStatus {
        if let Err(error) = self.ota.install_bundle(&self.download_file_path).await {
            let message = "Unable to install ota image".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::InvalidBaseImage(message), Some(ota_request));
        }

        debug!(
            "install_bundle done, last_error={:?}",
            self.last_error().await
        );

        if let Err(error) = self.ota.operation().await {
            let message = "Unable to get status of ota operation".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        info!("Waiting for signal...");
        let signal = self.ota.receive_completed().await;
        if signal.is_err() {
            let message = "Unable to receive the install completed event".to_string();
            error!("{message} : {}", signal.unwrap_err());
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        let signal = signal.unwrap();
        info!("Completed signal! {:?}", signal);

        match signal {
            0 => {
                info!("Update successful");

                let deployed_status = OtaStatus::Deployed(ota_request.clone());
                send_ota_event(publisher, &deployed_status).await;
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

    async fn rebooting(&self, ota_request: OtaRequest, publisher: &impl Publisher) -> OtaStatus {
        let rebooting_status = OtaStatus::Rebooting(ota_request.clone());
        send_ota_event(publisher, &rebooting_status).await;

        info!("Rebooting in 5 seconds");

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        #[cfg(not(test))]
        if let Err(error) = crate::power_management::reboot() {
            let message = "Unable to run reboot command".to_string();
            error!("{message} : {error}");
            return OtaStatus::Failure(OtaError::Internal(message), Some(ota_request.clone()));
        }

        rebooting_status
    }

    async fn success(&mut self) -> OtaStatus {
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

    async fn do_pending_ota(&self, state: &PersistentState) -> Result<(), OtaError> {
        const GOOD_STATE: &str = "good";

        let booted_slot = self.ota.boot_slot().await.map_err(|error| {
            let message = "Unable to identify the booted slot".to_string();
            error!("{message}: {error}");
            OtaError::Internal(message)
        })?;

        if state.slot == booted_slot {
            let message = "Unable to switch slot".to_string();
            return Err(OtaError::SystemRollback(message));
        }

        let primary_slot = self.ota.get_primary().await.map_err(|error| {
            let message = "Unable to get the current primary slot".to_string();
            error!("{message}: {error}");
            OtaError::Internal(message)
        })?;

        let (marked_slot, _) = self
            .ota
            .mark(GOOD_STATE, &primary_slot)
            .await
            .map_err(|error| {
                let message = "Unable to run marking slot operation".to_string();
                error!("{message}: {error}");
                OtaError::Internal(message)
            })?;

        if primary_slot != marked_slot {
            let message = "Unable to mark slot".to_string();
            Err(OtaError::Internal(message))
        } else {
            Ok(())
        }
    }

    pub async fn ensure_pending_ota_response(
        &mut self,
        sdk: &impl Publisher,
    ) -> Result<(), DeviceManagerError> {
        self.ota_status = OtaStatus::Rebooted;
        self.handle_ota_event(sdk, HashMap::new()).await
    }

    pub async fn ota_event(
        &mut self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        self.handle_ota_event(sdk, data).await
    }

    async fn handle_ota_event(
        &mut self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        let mut ota_status = self.ota_status.clone();

        loop {
            ota_status = match ota_status {
                OtaStatus::Init => self.acknowledged(sdk, data.clone()).await,
                OtaStatus::Acknowledged(ota_request) => self.downloading(ota_request, sdk).await,
                OtaStatus::Downloading(ota_request, _) => self.deploying(ota_request, sdk).await,
                OtaStatus::Deploying(ota_request) => self.deployed(ota_request, sdk).await,
                OtaStatus::Deployed(ota_request) => self.rebooting(ota_request, sdk).await,
                OtaStatus::Rebooted => self.success().await,
                OtaStatus::Error(ota_error, ota_request) => {
                    OtaStatus::Failure(ota_error, Some(ota_request))
                }
                OtaStatus::Rebooting(_)
                | OtaStatus::NoPendingOta
                | OtaStatus::Success(_)
                | OtaStatus::Failure(_, _) => break,
            };
        }

        if let Some(path) = self.get_update_file_path().to_str() {
            if let Err(e) = std::fs::remove_file(path) {
                error!("Unable to remove {}: {}", path, e);
            }
        }

        self.ota_status = OtaStatus::Init;

        match ota_status.clone() {
            OtaStatus::Failure(ota_error, _) => {
                send_ota_event(sdk, &ota_status).await?;
                return Err(DeviceManagerError::OtaError(ota_error));
            }
            OtaStatus::Success(_) => {
                send_ota_event(sdk, &ota_status).await?;
            }
            _ => {}
        }

        Ok(())
    }
}

async fn wget(
    url: &str,
    file_path: &str,
    request_uuid: &Uuid,
    sdk: &impl Publisher,
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
                .and_then(|size| if size <= 0 { None } else { Some(size) })
                .ok_or(OtaError::Network(format!(
                    "Unable to get content length from: {url}"
                )))?;

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

                let _ = send_ota_event(
                    sdk,
                    &OtaStatus::Downloading(
                        OtaRequest {
                            uuid: request_uuid.clone(),
                            url: "".to_string(),
                        },
                        progress,
                    ),
                )
                .await;
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
                ota_event.statusProgress = progress.clone();
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
            OtaStatus::Init | OtaStatus::NoPendingOta | OtaStatus::Rebooted => {}
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
    let ota_event = OtaEvent::from(ota_status);
    debug!("Sending ota response {:?}", ota_event);

    if ota_event.requestUUID.is_empty() {
        return Err(OtaError::Internal(
            "Unable to publish ota_event: request_uuid is empty".to_string(),
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use astarte_device_sdk::types::AstarteType;
    use httpmock::prelude::*;
    use tempdir::TempDir;
    use uuid::Uuid;

    use crate::data::MockPublisher;
    use crate::error::DeviceManagerError;
    use crate::ota::ota_handler::{
        wget, OtaError, OtaEvent, OtaHandler, OtaRequest, OtaStatus, PersistentState,
    };
    use crate::ota::rauc::BundleInfo;
    use crate::ota::MockOta;
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

        let ota_event: OtaEvent = OtaEvent::from(&OtaStatus::Acknowledged(ota_request.clone()));
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

        let ota_event = OtaEvent::from(&OtaStatus::Downloading(ota_request.clone(), 100));
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

        let ota_event = OtaEvent::from(&OtaStatus::Deploying(ota_request.clone()));
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

        let ota_event = OtaEvent::from(&OtaStatus::Deployed(ota_request.clone()));
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

        let ota_event = OtaEvent::from(&OtaStatus::Rebooting(ota_request.clone()));
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

        let ota_event = OtaEvent::from(&OtaStatus::Success(ota_request.clone()));
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
            OtaError::Request("Invalid data".to_string()),
            ota_request.clone(),
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
            OtaError::Request("Invalid data".to_string()),
            Some(ota_request.clone()),
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
            Some(ota_request.clone()),
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
            Some(ota_request.clone()),
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
            OtaError::Internal("system damage".to_string()),
            Some(ota_request.clone()),
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
            Some(ota_request.clone()),
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
            OtaError::SystemRollback("Unable to switch partition".to_string()),
            Some(ota_request.clone()),
        ));
        assert_eq!(expected_ota_event.status, ota_event.status);
        assert_eq!(expected_ota_event.statusCode, ota_event.statusCode);
        assert_eq!(expected_ota_event.message, ota_event.message);
        assert_eq!(expected_ota_event.requestUUID, ota_event.requestUUID);
    }

    #[tokio::test]
    async fn last_error_ok() {
        let mut ota = MockOta::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        ota.expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let last_error_result = ota_handler.last_error().await;

        assert!(last_error_result.is_ok());
        assert_eq!("Unable to deploy image", last_error_result.unwrap());
    }

    #[tokio::test]
    async fn last_error_fail() {
        let mut ota = MockOta::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        ota.expect_last_error().returning(|| {
            Err(DeviceManagerError::FatalError(
                "Unable to call last error".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let last_error_result = ota_handler.last_error().await;

        assert!(last_error_result.is_err());
        assert!(matches!(
            last_error_result.err().unwrap(),
            DeviceManagerError::FatalError(_)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_empty_data() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.acknowledged(&publisher, HashMap::new()).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_uuid() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        let data = HashMap::from([(
            "url".to_string(),
            AstarteType::String("http://instance.ota.bin".to_string()),
        )]);

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.acknowledged(&publisher, data).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_fail_data_with_one_key() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        let data = HashMap::from([
            (
                "url".to_string(),
                AstarteType::String("http://instance.ota.bin".to_string()),
            ),
            (
                "uuid".to_string(),
                AstarteType::String("bad_uuid".to_string()),
            ),
        ]);

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.acknowledged(&publisher, data).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn ota_event_fail_data_with_wrong_astarte_type() {
        let ota = MockOta::new();
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let mut data = HashMap::new();
        data.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        data.insert("uuid".to_owned(), AstarteType::Integer(0));

        let ota_status = ota_handler.acknowledged(&publisher, data).await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Request(_), _)
        ))
    }

    #[tokio::test]
    async fn try_to_acknowledged_success() {
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut publisher = MockPublisher::new();
        let ota = MockOta::new();

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
        ]);

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Acknowledged")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == uuid.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.acknowledged(&publisher, data).await;

        assert!(matches!(ota_status, OtaStatus::Acknowledged(_)))
    }

    #[tokio::test]
    async fn try_to_downloading_success() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        let ota_request = OtaRequest::default();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.downloading(ota_request, &publisher).await;

        assert!(matches!(ota_status, OtaStatus::Downloading(_, _)));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_request() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
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

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_5_wget() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
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

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Error")
                    && ota_event.statusCode.eq("NetworkError")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(4)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert_hits(5);

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Network(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_info() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
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
        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _),
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_ota_call_compatible() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Err(DeviceManagerError::FatalError("empty value".to_string())));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

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

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_compatible() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-arm".to_string()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let mut ota_request = OtaRequest::default();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

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

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_call_boot_slot() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        ota.expect_boot_slot().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let mut ota_request = OtaRequest::default();
        let request_uuid = ota_request.uuid.clone().to_string();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == request_uuid
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

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

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deploying_fail_write_state() {
        let mut publisher = MockPublisher::new();
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| {
            Err(DeviceManagerError::FatalError(
                "Unable to write".to_string(),
            ))
        });

        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        ota.expect_boot_slot().returning(|| Ok("A".to_string()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

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

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(ota_status, OtaStatus::Failure(OtaError::IO(_), _)));
    }

    #[tokio::test]
    async fn try_to_deploying_success() {
        let mut publisher = MockPublisher::new();
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut ota = MockOta::new();

        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        ota.expect_boot_slot().returning(|| Ok("A".to_string()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let mut ota_request = OtaRequest::default();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deploying")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == ota_request.uuid.clone().to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

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

        let ota_status = ota_handler.deploying(ota_request, &publisher).await;
        mock_ota_file_request.assert();

        assert!(matches!(ota_status, OtaStatus::Deploying(_)));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_install_bundle() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_install_bundle()
            .returning(|_| Err(DeviceManagerError::FatalError("install fail".to_string())));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler
            .deployed(OtaRequest::default(), &publisher)
            .await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_operation() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_install_bundle().returning(|_| Ok(()));
        ota.expect_operation().returning(|| {
            Err(DeviceManagerError::FatalError(
                "operation call fail".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler
            .deployed(OtaRequest::default(), &publisher)
            .await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_receive_completed() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_install_bundle().returning(|_| Ok(()));
        ota.expect_operation().returning(|| Ok("".to_string()));
        ota.expect_receive_completed().returning(|| {
            Err(DeviceManagerError::FatalError(
                "receive_completed call fail".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler
            .deployed(OtaRequest::default(), &publisher)
            .await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::Internal(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_fail_signal() {
        let publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_install_bundle().returning(|_| Ok(()));
        ota.expect_operation().returning(|| Ok("".to_string()));
        ota.expect_receive_completed().returning(|| Ok(-1));
        ota.expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler
            .deployed(OtaRequest::default(), &publisher)
            .await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::InvalidBaseImage(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_deployed_success() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();

        ota.expect_install_bundle().returning(|_| Ok(()));
        ota.expect_operation().returning(|| Ok("".to_string()));
        ota.expect_receive_completed().returning(|| Ok(0));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_request = OtaRequest::default();
        let request_uuid = ota_request.uuid.clone().to_string();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Deployed")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == request_uuid
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.deployed(ota_request, &publisher).await;

        assert!(matches!(ota_status, OtaStatus::Deployed(_)));
    }

    #[tokio::test]
    async fn try_to_rebooting_success() {
        let mut publisher = MockPublisher::new();
        let state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_request = OtaRequest::default();
        let request_uuid = ota_request.uuid.clone().to_string();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Rebooting")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 0
                    && ota_event.requestUUID == request_uuid
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let ota_status = ota_handler.rebooting(ota_request, &publisher).await;

        assert!(matches!(ota_status, OtaStatus::Rebooting(_)));
    }

    #[tokio::test]
    async fn try_to_success_no_pending_update() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        state_mock.expect_exists().returning(|| false);

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.success().await;

        assert!(matches!(ota_status, OtaStatus::NoPendingOta));
    }

    #[tokio::test]
    async fn try_to_success_fail_read_state() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let ota = MockOta::new();

        state_mock.expect_exists().returning(|| true);
        state_mock
            .expect_read()
            .returning(move || Err(DeviceManagerError::FatalError("Unable to read".to_string())));

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.success().await;

        assert!(matches!(ota_status, OtaStatus::Failure(OtaError::IO(_), _)));
    }

    #[tokio::test]
    async fn try_to_success_fail_pending_ota() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();
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

        ota.expect_boot_slot().returning(|| Ok("A".to_owned()));

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.success().await;

        assert!(matches!(
            ota_status,
            OtaStatus::Failure(OtaError::SystemRollback(_), _)
        ));
    }

    #[tokio::test]
    async fn try_to_success() {
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota = MockOta::new();
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

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let ota_status = ota_handler.success().await;

        assert!(matches!(ota_status, OtaStatus::Success(_)));
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_not_compatible() {
        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .returning(|_, _: &str, _: OtaEvent| Ok(()));

        let bundle_info = "rauc-demo-x86";
        let system_info = "rauc-demo-arm";

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut ota = MockOta::new();
        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: bundle_info.to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok(system_info.to_string()));

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

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

        let data = HashMap::from([
            ("url".to_string(), AstarteType::String(ota_url)),
            (
                "uuid".to_string(),
                AstarteType::String(Uuid::new_v4().to_string()),
            ),
        ]);

        let result = ota_handler.handle_ota_event(&publisher, data).await;
        mock_ota_file_request.assert();

        assert!(result.is_err());

        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::InvalidBaseImage(error_message) = ota_error {
                let expected_message = format!(
                    "bundle {} is not compatible with system {system_info}",
                    bundle_info
                );
                assert_eq!(expected_message, error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_install_completed_fail() {
        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .returning(|_, _: &str, _: OtaEvent| Ok(()));

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut ota = MockOta::new();
        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));
        ota.expect_operation().returning(|| Ok("".to_string()));
        ota.expect_receive_completed().returning(|| Ok(-1));
        ota.expect_install_bundle().returning(|_| Ok(()));
        ota.expect_boot_slot().returning(|| Ok("".to_owned()));
        ota.expect_last_error()
            .returning(|| Ok("Unable to deploy image".to_string()));
        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

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

        let data = HashMap::from([
            ("url".to_string(), AstarteType::String(ota_url)),
            (
                "uuid".to_string(),
                AstarteType::String(Uuid::new_v4().to_string()),
            ),
        ]);

        let result = ota_handler.handle_ota_event(&publisher, data).await;
        mock_ota_file_request.assert();

        assert!(result.is_err());

        if let DeviceManagerError::OtaError(ota_error) = result.err().unwrap() {
            if let OtaError::InvalidBaseImage(error_message) = ota_error {
                assert_eq!("Update failed with signal -1".to_string(), error_message)
            } else {
                panic!("Wrong OtaError type");
            }
        } else {
            panic!("Wrong DeviceManagerError type");
        }
    }

    #[tokio::test]
    async fn ensure_pending_ota_response_ota_fail() {
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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("A".to_owned()));
        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
            ota_status: OtaStatus::Init,
        };

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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;

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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("A".to_owned()));

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;

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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary().returning(|| {
            Err(DeviceManagerError::FatalError(
                "unable to call boot slot".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;

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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Err(DeviceManagerError::FatalError(
                "Unable to call mark function".to_string(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.1".to_owned(),
                "marked slot rootfs.1 as good".to_owned(),
            ))
        });

        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
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

        let mut ota = MockOta::new();
        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });
        let ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
        assert!(result.is_ok());
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

        let mut ota = MockOta::new();
        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_install_bundle()
            .returning(|_| Err(DeviceManagerError::FatalError("install fail".to_string())));

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

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
    async fn ota_event_success() {
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

        let mut ota = MockOta::new();
        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-x86".to_string()));

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_install_bundle().returning(|_: &str| Ok(()));
        ota.expect_operation().returning(|| Ok("".to_string()));
        ota.expect_receive_completed().returning(|| Ok(0));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let mut ota_handler = OtaHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "/tmp".to_string(),
            ota_status: OtaStatus::Init,
        };

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
        let result = ota_handler.ota_event(&publisher, ota_req_map).await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());

        let result = ota_handler.ensure_pending_ota_response(&publisher).await;

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
        let publisher = MockPublisher::new();

        let result = wget(
            server.url("/ota.bin").as_str(),
            ota_file.as_str(),
            &Uuid::new_v4(),
            &publisher,
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

        let mut publisher = MockPublisher::new();

        let result = wget(
            ota_url.as_str(),
            ota_file.as_str(),
            &uuid_request,
            &publisher,
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
        let publisher = MockPublisher::new();

        let result = wget(
            server.url("/ota.bin").as_str(),
            ota_file.as_str(),
            &Uuid::new_v4(),
            &publisher,
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

        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, ota_event: &OtaEvent| {
                ota_event.status.eq("Downloading")
                    && ota_event.statusCode.eq("")
                    && ota_event.statusProgress == 100
                    && ota_event.requestUUID == uuid_request.to_string()
            })
            .times(1)
            .returning(|_: &str, _: &str, _: OtaEvent| Ok(()));

        let result = wget(
            ota_url.as_str(),
            ota_file.as_str(),
            &uuid_request,
            &publisher,
        )
        .await;
        mock_ota_file_request.assert();

        assert!(result.is_ok());
    }
}
