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

use async_trait::async_trait;
use futures::stream::BoxStream;
#[cfg(test)]
use mockall::automock;

use crate::error::DeviceManagerError;
use crate::ota::rauc::BundleInfo;

mod ota_handle;
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

/// Provides the status of the deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployStatus {
    Progress(DeployProgress),
    Completed { signal: i32 },
}

/// Stream of the [`DeployStatus`] events
pub type ProgressStream = BoxStream<'static, Result<DeployStatus, DeviceManagerError>>;

/// A **trait** required for all SystemUpdate handlers that want to update a system.
#[cfg_attr(test, automock)]
#[async_trait]
pub trait SystemUpdate: Send + Sync {
    async fn install_bundle(&self, source: &str) -> Result<(), DeviceManagerError>;
    async fn last_error(&self) -> Result<String, DeviceManagerError>;
    async fn info(&self, bundle: &str) -> Result<BundleInfo, DeviceManagerError>;
    async fn operation(&self) -> Result<String, DeviceManagerError>;
    async fn compatible(&self) -> Result<String, DeviceManagerError>;
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
#[derive(thiserror::Error, Clone, Debug, PartialEq)]
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
    IO(String),
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
}

impl Default for DeployStatus {
    fn default() -> Self {
        DeployStatus::Progress(DeployProgress::default())
    }
}
