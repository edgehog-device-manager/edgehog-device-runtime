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
#[cfg(test)]
use mockall::automock;

use crate::error::DeviceManagerError;
use crate::ota::rauc::BundleInfo;

pub(crate) mod ota_handler;
pub(crate) mod rauc;

#[cfg_attr(test, automock)]
#[async_trait]
pub trait Ota: Send + Sync {
    async fn install_bundle(&self, source: &str) -> Result<(), DeviceManagerError>;
    async fn last_error(&self) -> Result<String, DeviceManagerError>;
    async fn info(&self, bundle: &str) -> Result<BundleInfo, DeviceManagerError>;
    async fn operation(&self) -> Result<String, DeviceManagerError>;
    async fn compatible(&self) -> Result<String, DeviceManagerError>;
    async fn boot_slot(&self) -> Result<String, DeviceManagerError>;
    async fn receive_completed(&self) -> Result<i32, DeviceManagerError>;
    async fn get_primary(&self) -> Result<String, DeviceManagerError>;
    async fn mark(
        &self,
        state: &str,
        slot_identifier: &str,
    ) -> Result<(String, String), DeviceManagerError>;
}
