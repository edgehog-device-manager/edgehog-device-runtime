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

use astarte_device_sdk::types::AstarteType;
use astarte_device_sdk::{error::Error as AstarteError, AstarteAggregate, AstarteDeviceDataEvent};
use async_trait::async_trait;
#[cfg(test)]
use mockall::automock;

pub mod astarte_device_sdk_lib;
pub mod astarte_message_hub_node;

#[cfg_attr(test, automock)]
#[async_trait]
pub trait Publisher: Send + Sync {
    async fn send_object<T: 'static>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), AstarteError>
    where
        T: AstarteAggregate + Send;
    //TODO add send_object_with_timestamp to this trait
    async fn send(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: AstarteType,
    ) -> Result<(), AstarteError>;
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait Subscriber {
    async fn on_event(&mut self) -> Result<AstarteDeviceDataEvent, AstarteError>;
}
