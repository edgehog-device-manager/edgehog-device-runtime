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

//! # Security Event Telemetry
//!
//! This module provides structured auditing and logging for system container lifecycle

use tracing::Level;

/// Represents a security device event.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEvent {
    /// Indicates that a container deployment process has been initialized.
    ContainerDeploymentInit = 0,
    /// Indicates that a container deployment process completed successfully.
    ContainerDeploymentOk = 1,
    /// Indicates that a container deployment process failed to complete.
    ContainerDeploymentFail = 2,
    /// Indicates that a container undeployment (removal) process has been initialized.
    ContainerUndeploymentInit = 3,
    /// Indicates that a container undeployment (removal) process completed successfully.
    ContainerUndeploymentOk = 4,
    /// Indicates that a container undeployment (removal) process failed to complete.
    ContainerUndeploymentFail = 5,
    /// Indicates that an offline or idle container is currently starting up.
    ContainerStartInit = 6,
    /// Indicates that a container successfully transitioned to a running state.
    ContainerStartOk = 7,
    /// Indicates that a container encountered a failure while attempting to start up.
    ContainerStartFail = 8,
    /// Indicates that a running container is currently shutting down or stopping.
    ContainerStopInit = 9,
    /// Indicates that a container has successfully stopped and cleared active run states.
    ContainerStopOk = 10,
    /// Indicates that a container encountered a failure while trying to halt execution.
    ContainerStopFail = 11,
}

impl SecurityEvent {
    /// Returns the original string description for the device event.
    fn description(&self) -> &'static str {
        match self {
            Self::ContainerDeploymentInit => "Container deployment started",
            Self::ContainerDeploymentOk => "Container deployment success",
            Self::ContainerDeploymentFail => "Container deployment failure",
            Self::ContainerUndeploymentInit => "Container undeployment started",
            Self::ContainerUndeploymentOk => "Container undeployment success",
            Self::ContainerUndeploymentFail => "Container undeployment failure",
            Self::ContainerStartInit => "Container starting",
            Self::ContainerStartOk => "Container started successfully",
            Self::ContainerStartFail => "Container failed to start",
            Self::ContainerStopInit => "Container stopping",
            Self::ContainerStopOk => "Container stopped successfully",
            Self::ContainerStopFail => "Container failed to stop",
        }
    }

    fn level(&self) -> Level {
        match self {
            Self::ContainerDeploymentFail
            | Self::ContainerUndeploymentFail
            | Self::ContainerStartFail
            | Self::ContainerStopFail => Level::ERROR,
            _ => Level::INFO,
        }
    }
}

/// Logs a structured security event to the specialized `"edgehog-security-event"` target.
///
/// # Structured Fields
/// Logs emitted by this function include the following key-value pairs for structured log collectors:
/// * `target`: Always explicitly set to `"edgehog-security-event"`.
/// * `id`: The `u16` numeric discriminant representing the specific security event type.
/// * `message`: The human-readable static string description of the event.
pub fn notify(event: SecurityEvent) {
    let id = event as u16;
    let message = event.description();

    match event.level() {
        Level::ERROR => tracing::error!(target:"edgehog-security-event", id, "{message}"),
        Level::WARN => tracing::warn!(target:"edgehog-security-event", id, "{message}"),
        Level::INFO => tracing::info!(target:"edgehog-security-event", id, "{message}"),
        Level::DEBUG => tracing::debug!(target:"edgehog-security-event", id, "{message}"),
        Level::TRACE => tracing::trace!(target:"edgehog-security-event", id, "{message}"),
    }
}
