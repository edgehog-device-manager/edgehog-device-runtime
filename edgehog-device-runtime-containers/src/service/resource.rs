// This file is part of Astarte.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;

use astarte_device_sdk::Client;
use tracing::instrument;

use super::{Id, Result};

use crate::Docker;

/// A resource in the nodes struct.
pub(crate) trait Resource: Into<NodeType> {
    fn dependencies(&self) -> Result<Vec<String>>;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeType {}

impl NodeType {
    #[instrument(skip_all)]
    pub(super) async fn store<D>(&mut self, _id: &Id, _device: &D) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
    }

    #[instrument(skip_all)]
    pub(super) async fn create<D>(&mut self, _id: &Id, _device: &D, _client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
    }

    #[instrument(skip_all)]
    pub(super) async fn start<D>(&mut self, id: &Id, device: &D, client: &Docker) -> Result<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
    }
}
