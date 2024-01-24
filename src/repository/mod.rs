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

use std::error::Error;

use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;

pub(crate) mod file_state_repository;

#[cfg_attr(test, automock(type Err = self::file_state_repository::FileStateError;))]
#[async_trait]
pub trait StateRepository<T: Send + Sync>: Send + Sync {
    type Err: Error;

    async fn write(&self, value: &T) -> Result<(), Self::Err>;
    async fn read(&self) -> Result<T, Self::Err>;
    async fn exists(&self) -> bool;
    async fn clear(&self) -> Result<(), Self::Err>;
}
