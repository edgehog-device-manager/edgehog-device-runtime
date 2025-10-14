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

use std::time::Duration;

use edgehog_proto::containers::v1::StatsResponse;
use edgehog_proto::tonic::Status;
use tokio::sync::mpsc;
use tracing::debug;
use tracing::error;
use uuid::Uuid;

use crate::service::containers::ContainerHandle;
use crate::service::containers::SharedContainerHandle;

use super::conv::convert_stats;

pub(crate) struct StatsSender {
    pub(crate) tx: mpsc::Sender<Result<StatsResponse, Status>>,
    pub(crate) ids: Vec<Uuid>,
    pub(crate) containers: SharedContainerHandle,
}

impl StatsSender {
    fn container_handle(&self) -> Result<&ContainerHandle, Status> {
        self.containers.get().ok_or_else(|| {
            error!("container service is not available");

            Status::unavailable("container service not available")
        })
    }
    async fn send_all(&self) -> eyre::Result<()> {
        let stats = self.container_handle()?.all_stats().await?;

        for (id, stat) in stats {
            let value = convert_stats(&id, stat);

            self.tx
                .send_timeout(Ok(value), Duration::from_secs(10))
                .await?;
        }

        Ok(())
    }

    pub(crate) async fn send(&self) -> eyre::Result<()> {
        if self.ids.is_empty() {
            self.send_all().await?;

            return Ok(());
        }

        for id in &self.ids {
            let res = self.container_handle()?.stats(id).await?;

            let stats = match res {
                Some(stats) => stats,
                None => {
                    debug!(%id, "missing stats");

                    continue;
                }
            };

            let value = convert_stats(id, stats);

            self.tx
                .send_timeout(Ok(value), Duration::from_secs(10))
                .await?;
        }

        Ok(())
    }
}
