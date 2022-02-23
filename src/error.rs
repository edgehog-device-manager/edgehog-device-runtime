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

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DeviceManagerError {
    #[error(transparent)]
    AstarteBuilderError(#[from] astarte_sdk::builder::AstarteBuilderError),

    #[error(transparent)]
    AstarteError(#[from] astarte_sdk::AstarteError),

    #[error(transparent)]
    ProcError(#[from] procfs::ProcError),

    #[error(transparent)]
    IOError(#[from] std::io::Error),

    #[error("urrecoverable error")]
    FatalError(String),
}
