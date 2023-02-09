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
use crate::ota::OTA;
use crate::power_management;
use crate::repository::file_state_repository::FileStateRepository;
use crate::repository::StateRepository;

#[derive(Serialize, Deserialize, Debug)]
struct PersistentState {
    uuid: Uuid,
    slot: String,
}

#[derive(thiserror::Error, Debug)]
pub enum OTAError {
    /// Bundle download failed
    #[error("OTAErrorNetwork")]
    Network,
    /// Deployment failed
    #[error("OTAErrorDeploy")]
    Deploy,
    /// OTA failed
    #[error("OTAFailed")]
    Failed,
}

#[derive(Debug)]
enum OTAStatus {
    InProgress,
    Done,
    Error(OTAError),
}

#[derive(AstarteAggregate)]
#[allow(non_snake_case)]
struct OTAResponse {
    uuid: String,
    status: String,
    statusCode: String,
}

impl OTAStatus {
    fn to_status_code(&self) -> (String, String) {
        match self {
            OTAStatus::InProgress => ("InProgress".to_string(), String::new()),
            OTAStatus::Done => ("Done".to_string(), String::new()),
            OTAStatus::Error(error) => ("Error".to_string(), error.to_string()),
        }
    }
}

pub struct OTAHandler<'a> {
    ota: Box<dyn OTA + 'a>,
    state_repository: Box<dyn StateRepository<PersistentState> + 'a>,
    download_file_path: String,
}

impl<'a> OTAHandler<'a> {
    pub async fn new(
        opts: &crate::DeviceManagerOptions,
    ) -> Result<OTAHandler<'a>, DeviceManagerError> {
        let ota = OTARauc::new().await?;

        Ok(OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(FileStateRepository::new(
                opts.store_directory.clone(),
                "state.json".to_owned(),
            )),
            download_file_path: opts.download_directory.clone(),
        })
    }

    pub async fn last_error(&self) -> Result<String, DeviceManagerError> {
        self.ota.last_error().await
    }

    fn get_update_file_path(&self) -> PathBuf {
        std::path::Path::new(&self.download_file_path).join("update.bin")
    }

    pub async fn ota_event(
        &mut self,
        sdk: &impl Publisher,
        data: HashMap<String, AstarteType>,
    ) -> Result<(), DeviceManagerError> {
        if !data.contains_key("url") || !data.contains_key("uuid") {
            return Err(DeviceManagerError::UpdateError(
                "Unable to find keys in OTA request".to_owned(),
            ));
        }

        if let (AstarteType::String(request_url), AstarteType::String(request_uuid_str)) =
            (&data["url"], &data["uuid"])
        {
            let request_uuid = Uuid::parse_str(request_uuid_str).map_err(|_| {
                DeviceManagerError::UpdateError("Unable to parse request_uuid".to_owned())
            })?;

            let result = match self.handle_ota_event(sdk, request_url, request_uuid).await {
                Err(err) => {
                    error!("Update failed!");
                    error!("{:?}", err);
                    error!("{:?}", self.last_error().await);

                    match err {
                        DeviceManagerError::OTAError(err) => self
                            .send_ota_response(sdk, &request_uuid, OTAStatus::Error(err))
                            .await
                            .ok(),
                        _ => self
                            .send_ota_response(
                                sdk,
                                &request_uuid,
                                OTAStatus::Error(OTAError::Failed),
                            )
                            .await
                            .ok(),
                    };
                    Err(DeviceManagerError::UpdateError(
                        "Unable to handle OTA event".to_owned(),
                    ))
                }
                _ => Ok(()),
            };

            // Installation error, we can remove the update file
            if let Some(path) = self.get_update_file_path().to_str() {
                if let Err(e) = std::fs::remove_file(path) {
                    error!("Unable to remove {}: {}", path, e);
                }
            }

            if result.is_ok() {
                info!("Update successful");
                info!("Rebooting in 5 seconds");

                tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                #[cfg(not(test))]
                power_management::reboot()?;
            }

            return result;
        } else {
            error!("Got bad data in OTARequest ({data:?})");
            Err(DeviceManagerError::UpdateError(
                "Got bad data in OTARequest".to_owned(),
            ))
        }
    }

    async fn handle_ota_event(
        &mut self,
        sdk: &impl Publisher,
        request_url: &str,
        request_uuid: Uuid,
    ) -> Result<(), DeviceManagerError> {
        info!("Got update event");

        self.send_ota_response(sdk, &request_uuid, OTAStatus::InProgress)
            .await?;

        let path = self.get_update_file_path();
        let path = path.to_str().ok_or_else(|| {
            DeviceManagerError::FatalError("wrong download file path".to_string())
        })?;

        #[cfg(not(test))]
        wget(request_url, path).await?;

        let bundle_info = self.ota.info(path).await?;
        debug!("bundle info: {:?}", bundle_info);

        let compatible = self.ota.compatible().await?;

        if bundle_info.compatible != compatible {
            error!(
                "bundle '{}' is not compatible with system '{}'",
                bundle_info.compatible, compatible
            );
            return Err(DeviceManagerError::UpdateError(
                "bundle is not compatible".to_owned(),
            ));
        }

        self.state_repository.write(&PersistentState {
            uuid: request_uuid.clone(),
            slot: self.ota.boot_slot().await?,
        })?;

        self.ota.install_bundle(path).await?;

        debug!(
            "install_bundle done, last_error={:?}",
            self.last_error().await
        );

        debug!("rauc operation = {}", self.ota.operation().await?);

        info!("Waiting for signal...");
        let signal = self.ota.receive_completed().await?;

        info!("Completed signal! {:?}", signal);

        match signal {
            0 => Ok(()),
            _ => {
                error!("Update failed with signal {signal}");
                Err(OTAError::Deploy.into())
            }
        }
    }

    pub async fn ensure_pending_ota_response(
        &self,
        sdk: &impl Publisher,
    ) -> Result<(), DeviceManagerError> {
        if self.state_repository.exists() {
            info!("Found pending update");
            let state = self.state_repository.read()?;
            let result_response = match self.do_pending_ota(&state).await {
                Ok(()) => {
                    info!("OTA successful");
                    self.send_ota_response(sdk, &state.uuid, OTAStatus::Done)
                        .await
                }
                Err(error) => {
                    warn!("OTA failed, error -> {:?}", error);
                    self.send_ota_response(sdk, &state.uuid, OTAStatus::Error(OTAError::Failed))
                        .await
                }
            };

            if result_response.is_err() {
                warn!(
                    "Unable to publish OTA response -> {:?}",
                    result_response.err()
                );
                //TODO Retry the publish response, maybe it should be done on the SDK side.
            }
        }

        Ok(())
    }

    async fn do_pending_ota(&self, state: &PersistentState) -> Result<(), DeviceManagerError> {
        const GOOD_STATE: &str = "good";

        let pending_ota_result = if state.slot != self.ota.boot_slot().await? {
            let primary_slot = self.ota.get_primary().await?;
            let (marked_slot, _) = self.ota.mark(GOOD_STATE, &primary_slot).await?;
            if primary_slot == marked_slot {
                Ok(())
            } else {
                Err(DeviceManagerError::UpdateError(
                    "Unable to mark slot".to_owned(),
                ))
            }
        } else {
            Err(DeviceManagerError::UpdateError(
                "Unable to switch slot".to_owned(),
            ))
        };

        self.state_repository.clear()?;

        pending_ota_result
    }

    async fn send_ota_response(
        &self,
        sdk: &impl Publisher,
        request_uuid: &Uuid,
        status: OTAStatus,
    ) -> Result<(), DeviceManagerError> {
        info!("Sending ota response {:?}", status);

        let (status, status_code) = status.to_status_code();

        sdk.send_object(
            "io.edgehog.devicemanager.OTAResponse",
            "/response",
            OTAResponse {
                uuid: request_uuid.to_string(),
                status,
                statusCode: status_code,
            },
        )
        .await?;

        Ok(())
    }
}

async fn wget(url: &str, file_path: &str) -> Result<(), DeviceManagerError> {
    if std::path::Path::new(file_path).exists() {
        std::fs::remove_file(file_path).expect(&format!("Unable to remove {}", file_path));
    }
    info!("Downloading {:?}", url);
    for i in 0..5 {
        let response = reqwest::get(url).await;

        match response {
            Ok(response) => {
                debug!("Writing {file_path}");
                let mut os_file = std::fs::File::create(&file_path)?;
                let mut content = std::io::Cursor::new(response.bytes().await?);
                std::io::copy(&mut content, &mut os_file)?;
                return Ok(());
            }
            Err(err) => {
                let wait = u64::pow(2, i);
                error!("Error downloading update: {err:?}");
                error!("Next attempt in {}s", wait);
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        }
    }

    Err(OTAError::Network.into())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use astarte_device_sdk::types::AstarteType;
    use astarte_device_sdk::AstarteError;
    use uuid::Uuid;

    use crate::data::MockPublisher;
    use crate::error::DeviceManagerError;
    use crate::ota::ota_handler::{OTAError, OTAHandler, OTAResponse, OTAStatus, PersistentState};
    use crate::ota::rauc::BundleInfo;
    use crate::ota::MockOTA;
    use crate::repository::MockStateRepository;

    #[test]
    fn ota_status() {
        assert_eq!(
            ("Error".to_owned(), "OTAFailed".to_owned()),
            OTAStatus::Error(OTAError::Failed).to_status_code()
        );
        assert_eq!(
            ("Error".to_owned(), "OTAErrorNetwork".to_owned()),
            OTAStatus::Error(OTAError::Network).to_status_code()
        );
        assert_eq!(
            ("Error".to_owned(), "OTAErrorDeploy".to_owned()),
            OTAStatus::Error(OTAError::Deploy).to_status_code()
        );
        assert_eq!(
            ("Done".to_owned(), "".to_owned()),
            OTAStatus::Done.to_status_code()
        );
        assert_eq!(
            ("InProgress".to_owned(), "".to_owned()),
            OTAStatus::InProgress.to_status_code()
        );
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_not_compatible() {
        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .returning(|_, _: &str, _: OTAResponse| Ok(()));

        let mut ota = MockOTA::new();
        ota.expect_info().returning(|_: &str| {
            Ok(BundleInfo {
                compatible: "rauc-demo-x86".to_string(),
                version: "1".to_string(),
            })
        });

        ota.expect_compatible()
            .returning(|| Ok("rauc-demo-arm".to_string()));

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let result = ota_handler
            .handle_ota_event(&publisher, "", Uuid::new_v4())
            .await;
        assert!(result.is_err());

        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "bundle is not compatible".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn handle_ota_event_bundle_install_bundle_fail() {
        let mut ota = MockOTA::new();

        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .returning(|_, _: &str, _: OTAResponse| Ok(()));

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

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_write().returning(|_| Ok(()));

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let result = ota_handler
            .handle_ota_event(&publisher, "", Uuid::new_v4())
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            DeviceManagerError::OTAError(OTAError::Deploy)
        ));
    }

    #[tokio::test]
    async fn ensure_pending_ota_response_ota_fail() {
        let mut ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        ota.expect_boot_slot().returning(|| Ok("A".to_owned()));

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.clone(),
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, response: &OTAResponse| {
                let status = OTAStatus::Error(OTAError::Failed).to_status_code();
                response.status.eq(&status.0)
                    && response.statusCode.eq(&status.1)
                    && response.uuid == uuid.to_string()
            })
            .returning(|_: &str, _: &str, _: OTAResponse| Ok(()));

        let result = ota_handler.ensure_pending_ota_response(&publisher).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ensure_pending_ota_response_ota_success() {
        let mut ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.to_owned(),
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, response: &OTAResponse| {
                let status = OTAStatus::Done.to_status_code();
                response.status.eq(&status.0)
                    && response.statusCode.eq(&status.1)
                    && response.uuid == uuid.to_string()
            })
            .returning(|_: &str, _: &str, _: OTAResponse| Ok(()));

        let result = ota_handler.ensure_pending_ota_response(&publisher).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn do_pending_ota_fail_marked_wrong_slot() {
        let mut ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.1".to_owned(),
                "marked slot rootfs.1 as good".to_owned(),
            ))
        });

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.clone(),
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to mark slot".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn do_pending_ota_fail_unknown_slot() {
        let mut ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        ota.expect_boot_slot().returning(|| Ok("A".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.2".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Err(DeviceManagerError::UpdateError(
                "No slot with class rootfs and name rootfs.2 found".to_owned(),
            ))
        });

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.clone(),
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to switch slot".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn do_pending_ota_success() {
        let mut ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let slot = "A";

        ota.expect_boot_slot().returning(|| Ok("B".to_owned()));
        ota.expect_get_primary()
            .returning(|| Ok("rootfs.0".to_owned()));
        ota.expect_mark().returning(|_: &str, _: &str| {
            Ok((
                "rootfs.0".to_owned(),
                "marked slot rootfs.0 as good".to_owned(),
            ))
        });

        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.clone(),
                slot: slot.to_owned(),
            })
        });

        state_mock.expect_clear().returning(|| Ok(()));

        let ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let state = ota_handler.state_repository.read().unwrap();
        let result = ota_handler.do_pending_ota(&state).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn ota_event_fail_empty_keys() {
        let ota = MockOTA::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let result = ota_handler
            .ota_event(&MockPublisher::new(), HashMap::new())
            .await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to find keys in OTA request".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn ota_event_fail_with_one_key() {
        let ota = MockOTA::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );

        let result = ota_handler
            .ota_event(&MockPublisher::new(), ota_req_map)
            .await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to find keys in OTA request".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn ota_event_fail_with_wrong_astarte_type() {
        let ota = MockOTA::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        ota_req_map.insert("uuid".to_owned(), AstarteType::Integer(0));

        let result = ota_handler
            .ota_event(&MockPublisher::new(), ota_req_map)
            .await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Got bad data in OTARequest".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn ota_event_fail_with_wrong_uuid() {
        let ota = MockOTA::new();
        let state_mock = MockStateRepository::<PersistentState>::new();

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String("bad_uuid".to_owned()),
        );

        let result = ota_handler
            .ota_event(&MockPublisher::new(), ota_req_map)
            .await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to parse request_uuid".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn ota_event_fail() {
        let ota = MockOTA::new();
        let uuid = Uuid::new_v4();
        let mut publisher = MockPublisher::new();
        publisher
            .expect_send_object()
            .returning(|_: &str, _: &str, _: OTAResponse| {
                Err(AstarteError::SendError("test".to_owned()))
            });

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );

        let state_mock = MockStateRepository::<PersistentState>::new();
        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let result = ota_handler.ota_event(&publisher, ota_req_map).await;

        assert!(result.is_err());
        match result.err().unwrap() {
            DeviceManagerError::UpdateError(val) => {
                assert_eq!(val, "Unable to handle OTA event".to_owned())
            }
            _ => {
                panic!("Wrong DeviceManagerError type");
            }
        };
    }

    #[tokio::test]
    async fn ota_event_success() {
        let mut ota = MockOTA::new();
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
        ota.expect_receive_completed().returning(|| Ok(0));

        let uuid = Uuid::new_v4();
        let slot = "A";
        let mut state_mock = MockStateRepository::<PersistentState>::new();
        state_mock.expect_exists().returning(|| true);
        state_mock.expect_read().returning(move || {
            Ok(PersistentState {
                uuid: uuid.to_owned(),
                slot: slot.to_owned(),
            })
        });
        state_mock.expect_write().returning(|_| Ok(()));
        state_mock.expect_clear().returning(|| Ok(()));

        let mut ota_handler = OTAHandler {
            ota: Box::new(ota),
            state_repository: Box::new(state_mock),
            download_file_path: "".to_owned(),
        };

        let mut publisher = MockPublisher::new();

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, response: &OTAResponse| {
                let status = OTAStatus::Done.to_status_code();
                response.status.eq(&status.0)
                    && response.statusCode.eq(&status.1)
                    && response.uuid == uuid.to_string()
            })
            .returning(|_: &str, _: &str, _: OTAResponse| Ok(()));

        publisher
            .expect_send_object()
            .withf(move |_: &str, _: &str, response: &OTAResponse| {
                let status = OTAStatus::InProgress.to_status_code();
                response.status.eq(&status.0)
                    && response.statusCode.eq(&status.1)
                    && response.uuid == uuid.to_string()
            })
            .returning(|_: &str, _: &str, _: OTAResponse| Ok(()));

        let mut ota_req_map = HashMap::new();
        ota_req_map.insert(
            "url".to_owned(),
            AstarteType::String("http://ota.bin".to_owned()),
        );
        ota_req_map.insert(
            "uuid".to_owned(),
            AstarteType::String(uuid.clone().to_string()),
        );
        let result = ota_handler.ota_event(&publisher, ota_req_map).await;

        assert!(result.is_ok());
    }
}
