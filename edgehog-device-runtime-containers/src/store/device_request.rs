// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use diesel::{delete, insert_or_ignore_into};
use diesel::{prelude::*, update};
use edgehog_store::conversions::SqlUuid;
use edgehog_store::db::HandleError;
use edgehog_store::models::QueryModel;
use edgehog_store::models::containers::container::ContainerMissingDeviceRequest;
use edgehog_store::models::containers::device_request::{
    DeviceRequest, DeviceRequestDeviceId, DeviceRequestOption,
};
use edgehog_store::models::containers::device_request::{
    DeviceRequestCapability, DeviceRequestStatus,
};
use edgehog_store::schema::containers::{
    container_device_requests, device_requests, device_requests_capabilities,
    device_requests_device_ids, device_requests_options,
};
use tracing::{error, instrument};
use uuid::Uuid;

use crate::requests::device_request::CreateDeviceRequest;

use super::{Result, StateStore, StoreError};

/// DTO to pass the values in a format usable by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredDeviceRequest {
    device_request: DeviceRequest,
    device_ids: Vec<String>,
    capabilities: Vec<Vec<String>>,
    options: HashMap<String, String>,
}

impl TryFrom<CreateDeviceRequest> for StoredDeviceRequest {
    type Error = StoreError;

    fn try_from(
        CreateDeviceRequest {
            id,
            deployment_id: _,
            driver,
            count,
            device_ids,
            capabilities,
            option_keys,
            option_values,
        }: CreateDeviceRequest,
    ) -> Result<Self> {
        let device_ids = Vec::from_iter(device_ids);
        let capabilities = capabilities
            .iter()
            .map(|json_array| {
                serde_json::from_str::<Vec<String>>(json_array).map_err(|error| {
                    error!(%error,"coulnd't convert capability JSON array");

                    StoreError::Conversion {
                        ctx: "couldn't deserialize JSON array for capabilities".to_string(),
                    }
                })
            })
            .collect::<Result<Vec<Vec<String>>>>()?;

        let options = option_keys.into_iter().zip(option_values).collect();

        Ok(StoredDeviceRequest {
            device_request: DeviceRequest {
                id: SqlUuid::new(id),
                status: DeviceRequestStatus::default(),
                count,
                driver: driver.into(),
            },
            device_ids,
            capabilities,
            options,
        })
    }
}

impl From<StoredDeviceRequest> for crate::docker::container::DeviceRequest {
    fn from(
        StoredDeviceRequest {
            device_request:
                DeviceRequest {
                    id: _,
                    status: _,
                    driver,
                    count,
                },
            device_ids,
            capabilities,
            options,
        }: StoredDeviceRequest,
    ) -> Self {
        Self {
            driver,
            count,
            device_ids: Vec::from_iter(device_ids),
            capabilities: capabilities.into_iter().map(Vec::from_iter).collect(),
            options,
        }
    }
}

impl StateStore {
    /// Stores the device request received from the CreateRequest
    #[instrument(skip_all, fields(%create_device_request.id))]
    pub(crate) async fn create_device_request(
        &self,
        create_device_request: CreateDeviceRequest,
    ) -> Result<()> {
        let dr = StoredDeviceRequest::try_from(create_device_request)?;

        let device_ids = dr
            .device_ids
            .into_iter()
            .map(|device_id| DeviceRequestDeviceId {
                device_request_id: dr.device_request.id,
                device_id,
            })
            .collect::<Vec<DeviceRequestDeviceId>>();
        let capabilities = dr
            .capabilities
            .into_iter()
            .zip(0i32..)
            .flat_map(|(capabilities, idx)| {
                capabilities
                    .into_iter()
                    .map(move |capability| DeviceRequestCapability {
                        device_request_id: dr.device_request.id,
                        idx,
                        capability,
                    })
            })
            .collect::<Vec<DeviceRequestCapability>>();
        let options = dr
            .options
            .into_iter()
            .map(|(name, value)| DeviceRequestOption {
                device_request_id: dr.device_request.id,
                name,
                value,
            })
            .collect::<Vec<DeviceRequestOption>>();

        self.handle
            .for_write(move |writer| {
                insert_or_ignore_into(device_requests::table)
                    .values(&dr.device_request)
                    .execute(writer)?;

                insert_or_ignore_into(device_requests_device_ids::table)
                    .values(&device_ids)
                    .execute(writer)?;

                insert_or_ignore_into(device_requests_capabilities::table)
                    .values(&capabilities)
                    .execute(writer)?;

                insert_or_ignore_into(device_requests_options::table)
                    .values(&options)
                    .execute(writer)?;

                insert_or_ignore_into(container_device_requests::table)
                    .values(ContainerMissingDeviceRequest::find_by_device_request(
                        &dr.device_request.id,
                    ))
                    .execute(writer)?;

                delete(ContainerMissingDeviceRequest::find_by_device_request(
                    &dr.device_request.id,
                ))
                .execute(writer)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Updates the state of a device_request
    #[instrument(skip(self))]
    pub(crate) async fn update_device_request_status(
        &self,
        device_request_id: Uuid,
        status: DeviceRequestStatus,
    ) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = update(DeviceRequest::find_id(&SqlUuid::new(device_request_id)))
                    .set(device_requests::status.eq(status))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    /// Deletes a [`DeviceRequest`]
    #[instrument(skip(self))]
    pub(crate) async fn delete_device_request(&self, device_request_id: Uuid) -> Result<()> {
        self.handle
            .for_write(move |writer| {
                let updated = delete(DeviceRequest::find_id(&SqlUuid::new(device_request_id)))
                    .execute(writer)?;

                HandleError::check_modified(updated, 1)?;

                Ok(())
            })
            .await?;

        Ok(())
    }

    #[instrument(skip(self))]
    pub(crate) async fn load_device_requests_to_publish(&self) -> Result<Vec<SqlUuid>> {
        let device_requests = self
            .handle
            .for_read(move |reader| {
                let device_requests = device_requests::table
                    .select(device_requests::id)
                    .filter(device_requests::status.eq(DeviceRequestStatus::Received))
                    .load::<SqlUuid>(reader)?;

                Ok(device_requests)
            })
            .await?;

        Ok(device_requests)
    }
}

pub fn load_stored_device_request(
    reader: &mut SqliteConnection,
    device_request: DeviceRequest,
) -> QueryResult<StoredDeviceRequest> {
    let device_ids = device_requests_device_ids::table
        .select(device_requests_device_ids::device_id)
        .filter(device_requests_device_ids::device_request_id.eq(&device_request.id))
        .load::<String>(reader)?;

    let idx_cap = device_requests_capabilities::table
        .select((
            device_requests_capabilities::idx,
            device_requests_capabilities::capability,
        ))
        .filter(device_requests_capabilities::device_request_id.eq(&device_request.id))
        .order_by((
            device_requests_capabilities::idx,
            device_requests_capabilities::capability,
        ))
        .load::<(i32, String)>(reader)?;

    let mut capabilities = Vec::new();
    if let Some((current, _)) = idx_cap.first() {
        let mut current = *current;
        let mut set = Vec::new();
        for (idx, cap) in idx_cap {
            if idx == current {
                set.push(cap);
            } else {
                capabilities.push(std::mem::take(&mut set));

                current = idx;
            }
        }

        if !set.is_empty() {
            capabilities.push(set);
        }
    }

    let options = device_requests_options::table
        .select((
            device_requests_options::name,
            device_requests_options::value,
        ))
        .filter(device_requests_options::device_request_id.eq(&device_request.id))
        .load_iter::<(String, String), _>(reader)?
        .collect::<QueryResult<HashMap<String, String>>>()?;

    Ok(StoredDeviceRequest {
        device_request,
        device_ids,
        capabilities,
        options,
    })
}

#[cfg(test)]
mod tests {
    use crate::requests::device_request::tests::create_device_request;

    use super::*;

    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    async fn find_device_request(store: &StateStore, id: Uuid) -> Option<StoredDeviceRequest> {
        store
            .handle
            .for_read(move |reader| {
                let id = SqlUuid::new(id);
                let Some(device_request) = DeviceRequest::find_id(&id)
                    .first::<DeviceRequest>(reader)
                    .optional()?
                else {
                    return Ok(None);
                };

                let stored = load_stored_device_request(reader, device_request)?;

                Ok(Some(stored))
            })
            .await
            .unwrap()
    }

    fn create_stored_device_request(id: Uuid) -> StoredDeviceRequest {
        StoredDeviceRequest {
            device_request: DeviceRequest {
                id: SqlUuid::new(id),
                status: DeviceRequestStatus::Received,
                driver: Some("nvidia".to_string()),
                count: -1,
            },
            device_ids: ["0", "1", "GPU-fef8089b-4820-abfc-e83e-94318197576e"]
                .map(str::to_string)
                .to_vec(),
            capabilities: vec![["compute", "gpu", "nvidia"].map(str::to_string).to_vec()],
            options: HashMap::from_iter(
                [("property1", "string"), ("property2", "string")]
                    .map(|(k, v)| (k.to_string(), v.to_string())),
            ),
        }
    }

    #[tokio::test]
    async fn should_store() {
        let tmp = TempDir::with_prefix("store_device_request").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let device_request_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let device_request = create_device_request(device_request_id, deployment_id);
        store.create_device_request(device_request).await.unwrap();

        let res = find_device_request(&store, device_request_id)
            .await
            .unwrap();

        let exp = create_stored_device_request(device_request_id);

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_update() {
        let tmp = TempDir::with_prefix("update_device_request").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let device_request_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let device_request = create_device_request(device_request_id, deployment_id);
        store.create_device_request(device_request).await.unwrap();

        store
            .update_device_request_status(device_request_id, DeviceRequestStatus::Published)
            .await
            .unwrap();

        let res = find_device_request(&store, device_request_id)
            .await
            .unwrap();

        let mut exp = create_stored_device_request(device_request_id);
        exp.device_request.status = DeviceRequestStatus::Published;

        assert_eq!(res, exp);
    }
}
