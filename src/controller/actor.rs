// This file is part of Edgehog.
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

//! Trait to generalize one task on the runtime.

use async_trait::async_trait;
use log::{debug, trace};
use stable_eyre::eyre::Context;
use tokio::sync::mpsc;

#[async_trait]
pub trait Actor: Sized {
    type Msg: Send + 'static;

    fn task() -> &'static str;

    async fn init(&mut self) -> stable_eyre::Result<()>;

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()>;

    async fn spawn(mut self, mut channel: mpsc::Receiver<Self::Msg>) -> stable_eyre::Result<()> {
        self.init()
            .await
            .wrap_err_with(|| format!("init for {} task failed", Self::task()))?;

        while let Some(msg) = channel.recv().await {
            trace!("message received for {} task", Self::task());

            self.handle(msg)
                .await
                .wrap_err_with(|| format!("handle for {} task failed", Self::task()))?;
        }

        debug!("task {} disconnected, closing", Self::task());

        Ok(())
    }
}
