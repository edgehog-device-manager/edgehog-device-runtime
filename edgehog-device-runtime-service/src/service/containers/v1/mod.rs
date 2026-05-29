// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

use std::result::Result;
use std::time::Duration;

use async_trait::async_trait;
use edgehog_containers::bollard::models::ContainerStateStatusEnum;
use edgehog_proto::containers::v1::containers_service_server::ContainersService;
use edgehog_proto::containers::v1::list_response::Info;
use edgehog_proto::containers::v1::{
    Container, ContainerId, ContainerState, GetRequest, GetResponse, ListInfo, ListInfoFull,
    ListInfoIds, ListInfoSummary, ListRequest, ListResponse, StartRequest, StatsRequest,
    StatsResponse, StopRequest,
};
use edgehog_proto::tonic::{self, Response, Status};
use tokio_stream::wrappers::ReceiverStream;
use tracing::error;
use uuid::Uuid;

use crate::service::EdgehogService;

use self::conv::{convert_container, convert_container_summary, parse_id};
use self::stats::StatsSender;

mod conv;
mod stats;

#[async_trait]
impl ContainersService for EdgehogService {
    type StatsStream = ReceiverStream<Result<StatsResponse, tonic::Status>>;

    async fn list(
        &self,
        request: tonic::Request<ListRequest>,
    ) -> Result<tonic::Response<ListResponse>, tonic::Status> {
        let inner = request.into_inner();
        let status = inner
            .container_state_filter()
            .map(|state| match state {
                ContainerState::Unspecified => ContainerStateStatusEnum::EMPTY,
                ContainerState::Created => ContainerStateStatusEnum::CREATED,
                ContainerState::Running => ContainerStateStatusEnum::RUNNING,
                ContainerState::Paused => ContainerStateStatusEnum::PAUSED,
                ContainerState::Restarting => ContainerStateStatusEnum::RESTARTING,
                ContainerState::Removing => ContainerStateStatusEnum::REMOVING,
                ContainerState::Exited => ContainerStateStatusEnum::EXITED,
                ContainerState::Dead => ContainerStateStatusEnum::DEAD,
            })
            .collect();

        let info = match inner.list_info() {
            ListInfo::Unspecified | ListInfo::Summary => {
                let containers = self
                    .container_handle()?
                    .list(status)
                    .await
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?
                    .into_iter()
                    .map(|(id, summary)| convert_container_summary(id, summary))
                    .collect();

                Info::Summary(ListInfoSummary { containers })
            }
            ListInfo::Ids => {
                let ids = self
                    .container_handle()?
                    .list_ids(status)
                    .await
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?
                    .into_iter()
                    .map(|(id, container_id)| ContainerId {
                        id: id.to_string(),
                        container_id,
                    })
                    .collect();

                Info::Ids(ListInfoIds { ids })
            }
            ListInfo::Full => {
                let containers = self
                    .container_handle()?
                    .get_all(status)
                    .await
                    .and_then(|containers| {
                        containers
                            .into_iter()
                            .map(|(id, container)| convert_container(&id, container))
                            .collect::<eyre::Result<Vec<Container>>>()
                    })
                    .map_err(|err| {
                        error!(error = format!("{err:#}"), "couldn't list containers");

                        Status::internal("couldn't list containers")
                    })?;

                Info::Full(ListInfoFull { containers })
            }
        };

        Ok(ListResponse { info: Some(info) }.into())
    }

    async fn get(
        &self,
        request: tonic::Request<GetRequest>,
    ) -> Result<tonic::Response<GetResponse>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let container = self
            .container_handle()?
            .get(id)
            .await
            .and_then(|container| {
                let Some(container) = container else {
                    return Ok(None);
                };

                convert_container(&id, container).map(Some)
            })
            .map_err(|err| {
                error!(error = format!("{err:#}"), "couldn't get container");

                Status::internal("couldn't get container")
            })?;

        Ok(GetResponse { container }.into())
    }

    async fn start(
        &self,
        request: tonic::Request<StartRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let started = self.container_handle()?.start(id).await.map_err(|err| {
            error!(error = format!("{err:#}"), "couldn't start the container");

            Status::internal("couldn't start the container")
        })?;

        if started.is_none() {
            return Err(Status::not_found("container doesn't exist"));
        }

        Ok(Response::new(()))
    }

    async fn stop(
        &self,
        request: tonic::Request<StopRequest>,
    ) -> Result<tonic::Response<()>, tonic::Status> {
        let inner = request.into_inner();
        let id = parse_id(&inner.id)?;

        let started = self.container_handle()?.stop(id).await.map_err(|err| {
            error!(error = format!("{err:#}"), "couldn't stop the container");

            Status::internal("couldn't stop the container")
        })?;

        if started.is_none() {
            return Err(Status::not_found("container doesn't exist"));
        }

        Ok(Response::new(()))
    }

    /// Stream container statistics.
    async fn stats(
        &self,
        request: tonic::Request<StatsRequest>,
    ) -> Result<tonic::Response<Self::StatsStream>, tonic::Status> {
        let StatsRequest {
            ids,
            interval_seconds,
            limit,
        } = request.into_inner();
        let ids = ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<Uuid>, Status>>()?;
        let interval = interval_seconds.unwrap_or(10);

        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let mut interval = tokio::time::interval(Duration::from_secs(interval));
        let containers = self.containers.clone();

        let sender = StatsSender {
            tx,
            ids,
            containers,
        };

        tokio::spawn(async move {
            let limit = limit.unwrap_or(u64::MAX);
            let range = (0..limit).take_while(|_| !sender.tx.is_closed());

            for _ in range {
                if let Err(err) = sender.send().await {
                    error!(error = format!("{err:#}"), "couldn't send stats");

                    break;
                }

                interval.tick().await;
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use edgehog_containers::local::MockContainerHandle;
    use mockall::{predicate, Sequence};
    use pretty_assertions::assert_eq;

    use crate::service::tests::mock_service;

    use super::*;

    #[tokio::test]
    async fn should_list_containers_empty() {
        let mut handle = MockContainerHandle::default();
        let mut seq = Sequence::new();

        handle
            .expect_list()
            .once()
            .in_sequence(&mut seq)
            .with(predicate::eq(Vec::new()))
            .returning(|_| Ok(HashMap::new()));

        let service = mock_service(handle);

        let req = ListRequest {
            list_info: None,
            container_state_filter: Vec::new(),
        };

        let res = service.list(tonic::Request::new(req)).await.unwrap();

        let inner = res.into_inner();

        let exp = ListResponse {
            info: Some(Info::Summary(ListInfoSummary {
                containers: Vec::new(),
            })),
        };
        assert_eq!(inner, exp);
    }
}
