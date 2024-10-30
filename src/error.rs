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

use astarte_device_sdk::event::FromEventError;
use thiserror::Error;

use crate::data::astarte_device_sdk_lib::DeviceSdkError;

#[derive(Error, Debug)]
pub enum DeviceManagerError {
    #[error(transparent)]
    Astarte(#[from] astarte_device_sdk::error::Error),

    #[error(transparent)]
    Procfs(#[from] procfs::ProcError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[cfg(all(feature = "zbus", target_os = "linux"))]
    #[error(transparent)]
    Zbus(#[from] zbus::Error),

    #[error("unrecoverable error ({0})")]
    Fatal(String),

    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[cfg(all(feature = "zbus", target_os = "linux"))]
    #[error(transparent)]
    Ota(#[from] crate::ota::OtaError),

    #[error("configuration file error")]
    ConfigFile(#[from] toml::de::Error),

    #[error("integer parse error")]
    ParseInt(#[from] std::num::ParseIntError),

    #[error("device SDK error")]
    DeviceSdk(#[from] DeviceSdkError),

    #[cfg(feature = "message-hub")]
    #[error("message hub error")]
    MessageHub(#[from] crate::data::astarte_message_hub_node::MessageHubError),

    #[error("the connection was closed")]
    Disconnected,

    #[error("couldn't connect to the store")]
    Store(#[from] crate::data::StoreError),

    #[error("couldn't convert device event")]
    FromEvent(#[from] FromEventError),

    #[cfg(feature = "containers")]
    #[error("container operation failed")]
    Containers(#[from] edgehog_containers::service::ServiceError),

    #[cfg(feature = "forwarder")]
    #[error("forwarder error")]
    Forwarder(#[from] crate::forwarder::ForwarderError),
}
