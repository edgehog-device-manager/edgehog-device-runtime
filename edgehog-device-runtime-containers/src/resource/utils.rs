// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use astarte_device_sdk::properties::PropAccess;
use tracing::error;
use uuid::Uuid;

use super::{ResourceError, Result};

const INTERFACE: &str = "io.edgehog.devicemanager.storage.File";

/// Gets the pathOnDevice of a file transfer
pub(crate) async fn file_store_device_path<D>(
    device: &D,
    resource: &'static str,
    id: Uuid,
    target_id: &str,
) -> Result<String>
where
    D: PropAccess + Send + Sync + 'static,
{
    let prop_path = format!("/{}/pathOnDevice", target_id);

    let data = device
        .property(INTERFACE, &prop_path)
        .await
        .map_err(ResourceError::AstarteProperty)?
        .ok_or_else(|| {
            error!("couldn't find file bind property");

            ResourceError::Missing { id, resource }
        })?;

    let path = String::try_from(data).map_err(|error| {
        error!(%error, "Astarte property has invalid type");

        ResourceError::Invalid {
            id,
            resource,
            ctx: "invalid property type",
        }
    })?;

    Ok(path)
}
