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

//! Astarte properties sent to list the local stored files.

use std::{
    borrow::Cow,
    collections::HashSet,
    path::{Path, PathBuf},
};

use astarte_device_sdk::{AstarteData, prelude::PropAccess};
use eyre::Context;
use tracing::{error, instrument, warn};

#[derive(Debug, Clone)]
pub(crate) struct StoredFile {
    id: String,
    path: PathBuf,
    size: i64,
}

impl StoredFile {
    pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.storage.File";
    const PATH_ENDPOINT: &str = "/pathOnDevice";
    const SIZE_ENDPOINT: &str = "/sizeBytes";

    pub(crate) fn create(id: String, path: PathBuf, size: u64) -> Self {
        Self {
            id,
            path,
            size: super::to_i64(size),
        }
    }

    pub(crate) async fn fetch_paths<C>(device: &C) -> eyre::Result<HashSet<String>>
    where
        C: PropAccess + Send + Sync + 'static,
    {
        let previous = device
            .interface_props(Self::INTERFACE)
            .await
            .wrap_err("can't retrieve previous properties")?;

        let set: HashSet<String> = previous
            .into_iter()
            .filter(|p| p.path.ends_with(Self::PATH_ENDPOINT))
            .filter_map(|p| match p.value {
                AstarteData::String(path) => {
                    Self::file_name_from_path(&path).map(|s| s.to_string())
                }
                d => {
                    warn!("expecting string in '{}' got {:?}", p.path, d);

                    None
                }
            })
            .collect();

        Ok(set)
    }

    #[instrument(skip_all)]
    pub(crate) async fn send<C>(self, device: &mut C) -> eyre::Result<()>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        let path = self.path.to_string_lossy().to_string();

        device
            .set_property(
                Self::INTERFACE,
                &format!("/{}{}", self.id, Self::PATH_ENDPOINT),
                path.into(),
            )
            .await?;

        device
            .set_property(
                Self::INTERFACE,
                &format!("/{}{}", self.id, Self::SIZE_ENDPOINT),
                self.size.into(),
            )
            .await
            .map_err(eyre::Error::from)
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) async fn deleted<C, S>(id: S, device: &mut C)
    where
        S: std::fmt::Display,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        if let Err(error) = device
            .unset_property(Self::INTERFACE, &format!("/{}{}", id, Self::PATH_ENDPOINT))
            .await
        {
            error!(%error, "can't send unset to astarte");
        }

        if let Err(error) = device
            .unset_property(Self::INTERFACE, &format!("/{}{}", id, Self::SIZE_ENDPOINT))
            .await
        {
            error!(%error, "can't send unset to astarte");
        }
    }

    fn file_name_from_path<'a>(path: &'a str) -> Option<Cow<'a, str>> {
        let path = Path::new(path);
        let file_name = path.file_name();

        if let Some(file_name) = file_name {
            Some(file_name.to_string_lossy())
        } else {
            warn!("invalid path stored in properties");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{AstarteData, store::SqliteStore, transport::mqtt::Mqtt};
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate::eq;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn stored_file() {
        let uuid = Uuid::new_v4();
        let mut path = PathBuf::new();
        let size = 10;
        path.push("./");
        path.push(uuid.to_string());

        let stored = StoredFile::create(uuid.to_string(), path.clone(), size);

        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        device
            .expect_set_property()
            .with(
                eq(<StoredFile>::INTERFACE),
                eq(format!("/{}{}", uuid, <StoredFile>::PATH_ENDPOINT)),
                eq(AstarteData::String(path.to_string_lossy().to_string())),
            )
            .returning(|_, _, _| Ok(()));

        device
            .expect_set_property()
            .with(
                eq(<StoredFile>::INTERFACE),
                eq(format!("/{}{}", uuid, <StoredFile>::SIZE_ENDPOINT)),
                eq(AstarteData::from(i64::try_from(size).unwrap())),
            )
            .returning(|_, _, _| Ok(()));

        stored.send(&mut device).await.unwrap();
    }

    #[tokio::test]
    async fn deleted_file() {
        let uuid = Uuid::new_v4();

        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        device
            .expect_unset_property()
            .with(
                eq(<StoredFile>::INTERFACE),
                eq(format!("/{}{}", uuid, <StoredFile>::PATH_ENDPOINT)),
            )
            .returning(|_, _| Ok(()));

        device
            .expect_unset_property()
            .with(
                eq(<StoredFile>::INTERFACE),
                eq(format!("/{}{}", uuid, <StoredFile>::SIZE_ENDPOINT)),
            )
            .returning(|_, _| Ok(()));

        StoredFile::deleted(uuid, &mut device).await;
    }
}
