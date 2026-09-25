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

use std::{collections::HashSet, path::PathBuf};

use astarte_device_sdk::prelude::PropAccess;
use eyre::{Context, OptionExt};
use tracing::{error, instrument, trace};

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

    fn path_endpoint(id: &str) -> String {
        format!("/{}{}", id, Self::PATH_ENDPOINT)
    }

    fn size_endpoint(id: &str) -> String {
        format!("/{}{}", id, Self::SIZE_ENDPOINT)
    }

    #[instrument]
    fn endpoint_id(path: &str) -> Option<&str> {
        trace!("extracting id");

        path.split('/').nth(1)
    }

    pub(crate) async fn fetch_path<C>(device: &C, id: &str) -> eyre::Result<PathBuf>
    where
        C: PropAccess,
    {
        let stored = device
            .property(Self::INTERFACE, &Self::path_endpoint(id))
            .await?
            .ok_or_eyre("no path property found")?;

        let path = String::try_from(stored).wrap_err("unexpected data type")?;

        Ok(PathBuf::from(path))
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
            .filter_map(|p| Self::endpoint_id(&p.path).map(|s| s.into()))
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
            .set_property(Self::INTERFACE, &Self::path_endpoint(&self.id), path.into())
            .await?;

        device
            .set_property(
                Self::INTERFACE,
                &Self::size_endpoint(&self.id),
                self.size.into(),
            )
            .await
            .map_err(eyre::Error::from)
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) async fn unset<C, S>(id: S, device: &mut C)
    where
        S: std::fmt::Display,
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        if let Err(error) = device
            .unset_property(Self::INTERFACE, &Self::path_endpoint(&id.to_string()))
            .await
        {
            error!(%error, "can't send unset to astarte");
        }

        if let Err(error) = device
            .unset_property(Self::INTERFACE, &Self::size_endpoint(&id.to_string()))
            .await
        {
            error!(%error, "can't send unset to astarte");
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::{AstarteData, store::SqliteStore, transport::mqtt::Mqtt};
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate::eq;
    use rstest::Context;
    use rstest::rstest;
    use uuid::Uuid;

    use crate::tests::with_insta;

    use super::*;

    #[tokio::test]
    async fn stored_file() {
        let uuid = Uuid::new_v4();
        let mut path = PathBuf::new();
        let size = 10;
        path.push("./");
        path.push(uuid.to_string());

        let stored = StoredFile::create(uuid.to_string(), path.clone(), size);

        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();

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

        let mut device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();

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

        StoredFile::unset(uuid, &mut device).await;
    }

    #[test]
    fn test_path_endpoint() {
        let endpoint = StoredFile::path_endpoint("testfile.txt");

        with_insta!({
            insta::assert_snapshot!(endpoint);
        });
    }

    #[test]
    fn test_size_endpoint() {
        let endpoint = StoredFile::size_endpoint("testfile.txt");

        with_insta!({
            insta::assert_snapshot!(endpoint);
        });
    }

    #[rstest]
    #[case(StoredFile::size_endpoint("testfile.txt"))]
    #[case(StoredFile::path_endpoint("testfile2.txt"))]
    fn test_endpoint_get_id(#[context] ctx: Context, #[case] endpoint: String) {
        let id = StoredFile::endpoint_id(&endpoint).unwrap();

        with_insta!({
            let name = format!("{}_{}", ctx.name, ctx.case.unwrap());

            insta::assert_snapshot!(name, id);
        });
    }
}
