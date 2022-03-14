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

use crate::error::DeviceManagerError;
use astarte_sdk::types::AstarteType;
use std::collections::HashMap;

/// get structured data for `io.edgehog.devicemanager.RuntimeInfo` interface
pub fn get_runtime_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    if let Ok(f) = std::env::var("CARGO_PKG_NAME") {
        ret.insert("/name".to_owned(), f.into());
    }

    if let Ok(f) = std::env::var("CARGO_PKG_HOMEPAGE") {
        ret.insert("/url".to_owned(), f.into());
    }

    if let Ok(f) = std::env::var("CARGO_PKG_VERSION") {
        ret.insert("/version".to_owned(), f.into());
    }

    ret.insert("/environment".to_owned(), "Rust".to_owned().into());

    Ok(ret)
}
