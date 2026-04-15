// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

//! Trait to generalize one task on the runtime.

use eyre::Context;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, instrument, trace};

pub trait Actor: Sized {
    type Msg: Send + 'static;

    fn task() -> &'static str;

    fn init(&mut self) -> impl Future<Output = eyre::Result<()>> + Send;

    fn handle(&mut self, msg: Self::Msg) -> impl Future<Output = eyre::Result<()>> + Send;

    #[instrument(skip_all, fields(task = Self::task()))]
    async fn run(
        mut self,
        mut channel: mpsc::Receiver<Self::Msg>,
        cancel: CancellationToken,
    ) -> eyre::Result<()> {
        self.init().await.wrap_err("init task failed")?;

        while let Some(msg) = cancel.run_until_cancelled(channel.recv()).await.flatten() {
            trace!("message received");

            self.handle(msg).await.wrap_err("handle failed")?;
        }

        debug!("task disconnected, closing");

        Ok(())
    }
}

/// Persisted jobs for an actor
#[cfg(feature = "file-transfer")]
pub trait Persisted: Sized {
    type Msg: Send + 'static;

    fn task() -> &'static str;

    fn init(&mut self) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Queue struct
    fn queue(&self) -> &crate::jobs::Queue;

    /// Notify to wake a worker thead
    fn workers(&self) -> &tokio::sync::Notify;

    /// Validate a msg and converts it into a Job.
    fn validate_job(
        &mut self,
        msg: &Self::Msg,
    ) -> impl Future<Output = eyre::Result<edgehog_store::models::job::Job>> + Send;

    /// Handles the case when a job cannot be queued
    fn fail_job(
        &mut self,
        msg: &Self::Msg,
        report: eyre::Report,
    ) -> impl Future<Output = ()> + Send;

    /// Handles the case when a job cannot be queued
    fn handle_backpressure(&mut self, msg: &Self::Msg) -> impl Future<Output = ()> + Send;

    #[instrument(skip_all)]
    async fn handle(&mut self, msg: Self::Msg) {
        trace!("message received");

        let job = match self.validate_job(&msg).await {
            Ok(job) => job,
            Err(error) => {
                tracing::error!(%error, "couldn't queue the job");

                self.fail_job(&msg, error).await;

                return;
            }
        };

        if let Err(error) = self.queue().insert_job(job).await {
            tracing::error!(%error, "couldn't queue the job");

            // TODO: time-out
            self.handle_backpressure(&msg).await;
        }

        debug!("task queued");

        self.workers().notify_one();

        debug!("task notified");
    }

    #[instrument(skip_all, fields(task = Self::task()))]
    async fn run(
        mut self,
        mut channel: mpsc::Receiver<Self::Msg>,
        cancel: CancellationToken,
    ) -> eyre::Result<()> {
        self.init().await.wrap_err("init failed")?;

        while let Some(msg) = cancel.run_until_cancelled(channel.recv()).await.flatten() {
            trace!("message received");

            self.handle(msg).await;
        }

        debug!("task disconnected, closing");

        Ok(())
    }
}
