// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Persistent stores of the request issued by Astarte and resources created.

use std::ops::Not;

use edgehog_store::db::{self, HandleError};

use crate::requests::{container::RestartPolicyError, BindingError};

mod container;
mod deployment;
mod device_mapping;
mod image;
mod network;
mod volume;

type Result<T> = std::result::Result<T, StoreError>;

/// Error returned by the [`StateStore`].
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum StoreError {
    /// couldn't parse {ctx} key value {value}
    ParseKeyValue {
        /// Key that couldn't be parsed
        ctx: &'static str,
        /// Value that couldn't be parsed
        value: String,
    },
    /// couldn't parse container port bindings
    PortBinding(#[from] BindingError),
    /// couldn't parse the container restart policy
    RestartPolicy(#[from] RestartPolicyError),
    /// database operation failed
    Handle(#[from] HandleError),
    /// conversion failed, {ctx}
    Conversion {
        /// Context of the error
        ctx: String,
    },
}

/// Handle to persist the state.
///
/// It's a wrapper around the SQLITE database.
#[derive(Debug)]
pub struct StateStore {
    handle: db::Handle,
}

impl StateStore {
    /// Creates a new state store
    pub fn new(handle: db::Handle) -> Self {
        Self { handle }
    }

    /// Clone the underlying handle lazily
    pub fn clone_lazy(&self) -> Self {
        Self {
            handle: self.handle.clone_lazy(),
        }
    }
}

#[allow(dead_code)]
fn split_key_value(value: &str) -> Option<(&str, Option<&str>)> {
    value.split_once('=').and_then(|(k, v)| {
        if k.is_empty() {
            return None;
        }

        let v = v.is_empty().not().then_some(v);

        Some((k, v))
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::requests::{
        container::CreateContainer, deployment::CreateDeployment, image::CreateImage,
        network::CreateNetwork, volume::CreateVolume, ReqUuid, VecReqUuid,
    };

    use super::*;

    #[test]
    fn should_parse_key_value() {
        let cases = [
            ("device=tmpfs", ("device", Some("tmpfs"))),
            ("o=size=100m,uid=1000", ("o", Some("size=100m,uid=1000"))),
            ("type=tmpfs", ("type", Some("tmpfs"))),
        ];

        for (case, exp) in cases {
            let res = split_key_value(case).unwrap();

            assert_eq!(res, exp);
        }
    }

    #[tokio::test]
    async fn should_create_missing() {
        let tmp = TempDir::with_prefix("create_full_deployment").unwrap();
        let db_file = tmp.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let image_id = Uuid::new_v4();
        let volume_id = ReqUuid(Uuid::new_v4());
        let network_id = ReqUuid(Uuid::new_v4());
        let device_mapping_id = ReqUuid(Uuid::new_v4());
        let container_id = ReqUuid(Uuid::new_v4());
        let deployment_id = ReqUuid(Uuid::new_v4());

        let deployment = CreateDeployment {
            id: deployment_id,
            containers: VecReqUuid(vec![container_id]),
        };
        store.create_deployment(deployment).await.unwrap();

        let container = CreateContainer {
            id: container_id,
            deployment_id: ReqUuid(image_id),
            image_id: ReqUuid(image_id),
            network_ids: VecReqUuid(vec![network_id]),
            volume_ids: VecReqUuid(vec![volume_id]),
            device_mapping_ids: VecReqUuid(vec![device_mapping_id]),
            hostname: "database".to_string(),
            restart_policy: "unless-stopped".to_string(),
            env: ["POSTGRES_USER=user", "POSTGRES_PASSWORD=password"]
                .map(str::to_string)
                .to_vec(),
            binds: vec!["/var/lib/postgres".to_string()],
            network_mode: "bridge".to_string(),
            port_bindings: vec!["5432:5432".to_string()],
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            cap_add: vec!["CAP_CHOWN".to_string()],
            cap_drop: vec!["CAP_KILL".to_string()],
            cpu_period: 1000,
            cpu_quota: 100,
            cpu_realtime_period: 1000,
            cpu_realtime_runtime: 100,
            memory: 4096,
            memory_reservation: 1024,
            memory_swap: 8192,
            memory_swappiness: 50,
            volume_driver: "local".to_string().into(),
            storage_opt: vec!["size=1024k".to_string()],
            read_only_rootfs: true,
            tmpfs: vec!["/run=rw,noexec,nosuid,size=65536k".to_string()],
            privileged: false,
        };
        store.create_container(Box::new(container)).await.unwrap();

        let network = CreateNetwork {
            id: network_id,
            deployment_id,
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        };
        store.create_network(network).await.unwrap();

        let volume = CreateVolume {
            id: volume_id,
            deployment_id,
            driver: "local".to_string(),
            options: ["device=tmpfs", "o=size=100m,uid=1000", "type=tmpfs"]
                .map(str::to_string)
                .to_vec(),
        };
        store.create_volume(volume).await.unwrap();

        let image = CreateImage {
            id: ReqUuid(image_id),
            deployment_id,
            reference: "postgres:15".to_string(),
            registry_auth: String::new(),
        };
        store.create_image(image).await.unwrap();
    }
}
