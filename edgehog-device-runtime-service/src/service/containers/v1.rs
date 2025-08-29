// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use async_trait::async_trait;
use edgehog_proto::containers::v1::containers_service_server::ContainersService;
use edgehog_proto::containers::v1::{
    GetRequest, GetResponse, ListRequest, ListResponse, StartRequest, StatsRequest, StatsResponse,
    StopRequest,
};
use edgehog_proto::tonic;

use crate::service::EdgehogService;

#[async_trait]
impl ContainersService for EdgehogService {
    type StatsStream = Box<
        dyn tokio_stream::Stream<Item = std::result::Result<StatsResponse, tonic::Status>>
            + Send
            + Sync
            + Unpin
            + 'static,
    >;

    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> std::result::Result<tonic::Response<ListResponse>, tonic::Status> {
        todo!()
    }

    async fn get(
        &self,
        request: tonic::Request<GetRequest>,
    ) -> std::result::Result<tonic::Response<GetResponse>, tonic::Status> {
        todo!()
    }

    async fn start(
        &self,
        request: tonic::Request<StartRequest>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        todo!()
    }

    async fn stop(
        &self,
        request: tonic::Request<StopRequest>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        todo!()
    }

    /// Stream container statistics.
    async fn stats(
        &self,
        request: tonic::Request<StatsRequest>,
    ) -> std::result::Result<tonic::Response<Self::StatsStream>, tonic::Status> {
        todo!()
    }
}
