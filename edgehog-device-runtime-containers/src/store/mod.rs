// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

//! Persistent stores of the request issued by Astarte and resources created.

use std::ops::Not;

use edgehog_store::db::{self, HandleError};

use crate::requests::BindingError;

pub(crate) mod container;
pub(crate) mod deployment;
pub(crate) mod device_mapping;
pub(crate) mod device_request;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

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
#[derive(Debug, Clone)]
pub struct StateStore {
    handle: db::Handle,
}

impl StateStore {
    /// Creates a new state store
    pub fn new(handle: db::Handle) -> Self {
        Self { handle }
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

    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;
    use crate::requests::{ReqUuid, VecReqUuid, deployment::CreateDeployment};

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

        let deployment_id = ReqUuid(Uuid::new_v4());
        let image = create_image_req(deployment_id.0);
        let volume = create_volume_req(deployment_id.0);
        let network = create_network_req(deployment_id.0);
        let device_mapping = create_device_mapping_req(deployment_id.0);
        let device_request = create_device_request(deployment_id.0);
        let container = create_container_req(
            deployment_id.0,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
        );

        let deployment = CreateDeployment {
            id: deployment_id,
            containers: VecReqUuid(vec![container.id]),
        };
        store.create_deployment(deployment).await.unwrap();

        store.create_container(Box::new(container)).await.unwrap();

        store.create_network(network).await.unwrap();

        store.create_volume(volume).await.unwrap();

        store.create_image(image).await.unwrap();
    }
}
