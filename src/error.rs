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

use crate::data::astarte_device_sdk_lib::DeviceSdkError;

#[derive(Error, Debug)]
pub enum DeviceManagerError {
    #[error(transparent)]
    AstarteError(#[from] astarte_device_sdk::error::Error),
    #[error(transparent)]
    ProcError(#[from] procfs::ProcError),

    #[error(transparent)]
    IOError(#[from] std::io::Error),

    #[error(transparent)]
    ZbusError(#[from] zbus::Error),

    #[error("unrecoverable error ({0})")]
    FatalError(String),

    #[error(transparent)]
    ReqwestError(#[from] reqwest::Error),

    #[error(transparent)]
    SerdeJsonError(#[from] serde_json::Error),

    #[error(transparent)]
    OtaError(#[from] crate::ota::OtaError),

    #[error("configuration file error")]
    ConfigFileError(#[from] toml::de::Error),

    #[error("integer parse error")]
    ParseIntError(#[from] std::num::ParseIntError),

    #[error("device SDK error")]
    DeviceSdk(#[from] DeviceSdkError),

    #[cfg(feature = "message-hub")]
    #[error("Message hub error")]
    MessageHub(#[from] crate::data::astarte_message_hub_node::MessageHubError),

    #[error("the connection was closed")]
    Disconnected,

    #[cfg(feature = "forwarder")]
    #[error("Forwarder error")]
    Forwarder(#[from] crate::forwarder::ForwarderError),
}
