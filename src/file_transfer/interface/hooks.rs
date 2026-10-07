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

use std::{io, process::ExitStatus};

use astarte_device_sdk::{AstarteData, IntoAstarteObject, aggregate::AstarteObject};
use tracing::{instrument, trace};

use crate::file_transfer::{
    errno::to_errno, hooks::HookExecutor, interface::status::TransferDirection,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum HookPhase {
    Pre,
    Post,
}

impl std::fmt::Display for HookPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookPhase::Pre => f.write_str("pre"),
            HookPhase::Post => f.write_str("post"),
        }
    }
}

impl From<HookPhase> for AstarteData {
    fn from(value: HookPhase) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
pub(crate) struct HookResponse {
    id: String,
    #[astarte_object(rename = "type")]
    ty: TransferDirection,
    hook_phase: HookPhase,
    code: i32,
    status_code: i32,
    message: String,
}

impl HookResponse {
    const INTERFACE: &str = "io.edgehog.devicemanager.fileTransfer.HookResponse";
    const NONE: i32 = -1;

    pub(crate) fn io_error(executor: &HookExecutor, phase: HookPhase, error: &io::Error) -> Self {
        Self {
            id: executor.id.clone(),
            ty: executor.direction,
            code: to_errno(error.kind()),
            message: error.to_string(),
            hook_phase: phase,
            status_code: Self::NONE,
        }
    }

    pub(crate) fn exit_status(
        executor: &HookExecutor,
        phase: HookPhase,
        status: ExitStatus,
    ) -> Self {
        Self {
            id: executor.id.clone(),
            ty: executor.direction,
            code: Self::NONE,
            message: String::new(),
            hook_phase: phase,
            status_code: status.code().unwrap_or(Self::NONE),
        }
    }

    #[instrument(skip_all)]
    pub(crate) async fn send<C>(self, device: &mut C) -> eyre::Result<()>
    where
        C: astarte_device_sdk::Client + Send + Sync + 'static,
    {
        trace!("sending response");

        device
            .send_object(Self::INTERFACE, "/request", AstarteObject::try_from(self)?)
            .await
            .map_err(eyre::Error::from)
    }
}
