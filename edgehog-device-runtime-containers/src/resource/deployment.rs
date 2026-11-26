// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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

use std::collections::BTreeMap;

use edgehog_store::models::containers::deployment::DeploymentStatus;
use rustc_hash::FxHashSet;
use uuid::Uuid;

use crate::properties::{
    AvailableProp, Client,
    deployment::{AvailableDeployment, DeploymentStatus as PropertyStatus},
};

use super::{Context, Resource, Result};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Deployment {
    pub(crate) containers: BTreeMap<i64, Uuid>,
    pub(crate) images: FxHashSet<Uuid>,
    pub(crate) volumes: FxHashSet<Uuid>,
    pub(crate) networks: FxHashSet<Uuid>,
    pub(crate) device_mappings: FxHashSet<Uuid>,
    pub(crate) device_requests: FxHashSet<Uuid>,
    pub(crate) file_binds: FxHashSet<Uuid>,
    pub(crate) env_files: FxHashSet<Uuid>,
}

impl<D> Resource<D> for Deployment
where
    D: Client + Send + Sync + 'static,
{
    async fn publish(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableDeployment::new(&ctx.id)
            .send(ctx.device, PropertyStatus::Stopped)
            .await?;

        ctx.store
            .update_deployment_status(ctx.id, DeploymentStatus::Stopped)
            .await?;

        Ok(())
    }

    async fn unset(ctx: &mut Context<'_, D>) -> Result<()> {
        AvailableDeployment::new(&ctx.id).unset(ctx.device).await?;

        ctx.store.delete_deployment(ctx.id).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use edgehog_store::conversions::SqlUuid;
    use edgehog_store::db;
    use mockall::Sequence;
    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::Docker;
    use crate::properties::deployment::tests::{
        expect_available_deployment_send, expect_available_deployment_unset,
    };
    use crate::requests::deployment::CreateDeployment;
    use crate::requests::{ReqUuid, VecReqUuid};
    use crate::store::StateStore;

    use super::*;

    struct TestFixture {
        _tempdir: TempDir,
        client: Docker,
        device: MockDeviceClient<Mqtt<SqliteStore, PairingApi>>,
        store: StateStore,
        id: Uuid,
    }

    async fn setup_test_fixture() -> TestFixture {
        let tempdir = TempDir::new().unwrap();
        let client = Docker::connect().await.unwrap();
        let device = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();

        let db_file = tempdir.path().join("state.db");
        let handle = db::Handle::open(&db_file).await.unwrap();
        let store = StateStore::new(handle);

        let id = Uuid::new_v4();
        let deployment = CreateDeployment {
            id: ReqUuid(id),
            containers: VecReqUuid(Vec::new()),
        };
        store.create_deployment(deployment).await.unwrap();

        TestFixture {
            _tempdir: tempdir,
            client,
            device,
            store,
            id,
        }
    }

    #[tokio::test]
    async fn should_publish() {
        let mut fixture = setup_test_fixture().await;
        let mut seq = Sequence::new();

        expect_available_deployment_send(
            &mut fixture.device,
            &mut seq,
            &fixture.id,
            PropertyStatus::Stopped,
        );

        let mut ctx = Context {
            id: fixture.id,
            store: &mut fixture.store,
            device: &mut fixture.device,
            client: &fixture.client,
        };

        Deployment::publish(&mut ctx).await.unwrap();

        let stopped = fixture
            .store
            .load_deployments_in(DeploymentStatus::Stopped)
            .await
            .unwrap();
        assert_eq!(stopped, vec![SqlUuid::new(fixture.id)]);
    }

    #[tokio::test]
    async fn should_unset() {
        let mut fixture = setup_test_fixture().await;
        let mut seq = Sequence::new();

        expect_available_deployment_unset(&mut fixture.device, &mut seq, &fixture.id);

        let mut ctx = Context {
            id: fixture.id,
            store: &mut fixture.store,
            device: &mut fixture.device,
            client: &fixture.client,
        };

        Deployment::unset(&mut ctx).await.unwrap();

        assert!(
            fixture
                .store
                .load_deployment_containers(fixture.id)
                .await
                .unwrap()
                .is_none()
        );
    }
}
