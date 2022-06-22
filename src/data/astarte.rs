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

use astarte_sdk::builder::AstarteOptions;
use astarte_sdk::{AstarteError, AstarteSdk};
use async_trait::async_trait;
use serde::Serialize;

use crate::data::Publisher;

#[derive(Clone)]
pub struct Astarte {
    pub device_sdk: AstarteSdk,
}

#[async_trait]
impl Publisher for Astarte {
    async fn send_object<T>(
        &self,
        interface_name: &str,
        interface_path: &str,
        data: T,
    ) -> Result<(), AstarteError>
    where
        T: Serialize + Send,
    {
        self.device_sdk
            .send_object(interface_name, interface_path, data)
            .await
    }
}

impl Astarte {
    pub async fn new(opts: &AstarteOptions) -> Result<Astarte, AstarteError> {
        let device = AstarteSdk::new(&opts).await?;
        Ok(Astarte { device_sdk: device })
    }
}
