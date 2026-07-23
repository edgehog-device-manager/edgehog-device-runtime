// This file is part of Edgehog.
//
// Copyright 2022, 2026 SECO Mind Srl
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

use std::task::Poll;

use eyre::Context;
use futures::stream::FusedStream;
use futures::{Stream, StreamExt, TryStreamExt, future, ready};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, instrument, warn};
use zbus::proxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

use crate::ota::config::RaucDbus;
use crate::ota::{DeployProgress, DeployStatus, SystemUpdate};

use super::ProgressStream;
use super::config::OtaConfig;

#[derive(DeserializeDict, SerializeDict, Type, Debug)]
#[zvariant(signature = "dict")]
pub struct SlotStatus {
    #[zvariant(rename = "boot-status")]
    boot_status: Option<String>,
    bootname: Option<String>,
    class: String,
    device: String,
    state: String,
    #[zvariant(rename = "type")]
    type_: String,
}

#[derive(Debug, Deserialize, Serialize, Type)]
#[zvariant(signature = "(sa{sv})")]
pub struct Slot {
    name: String,
    data: SlotStatus,
}

#[derive(Debug, Deserialize, Serialize, Type)]
#[zvariant(signature = "(ss)")]
pub struct BundleInfo {
    pub compatible: String,
    pub version: String,
}

#[proxy(
    interface = "de.pengutronix.rauc.Installer",
    default_service = "de.pengutronix.rauc",
    default_path = "/"
)]
pub trait Rauc {
    /// Triggers the installation of a bundle. This method call is non-blocking.
    /// After completion, the “Completed” signal will be emitted.
    fn install_bundle(
        &self,
        source: &str,
        args: std::collections::HashMap<String, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<()>;

    /// Provides bundle info.
    fn info(&self, bundle: &str) -> zbus::Result<BundleInfo>;

    /// Keeps a slot bootable (state == “good”),
    /// makes it unbootable (state == “bad”)
    /// or explicitly activates it for the next boot (state == “active”).
    fn mark(&self, state: &str, slot_identifier: &str) -> zbus::Result<(String, String)>;

    /// Access method to get all slots’ status.
    fn get_slot_status(&self) -> zbus::Result<Vec<Slot>>; //a(sa{sv})

    /// Get the current primary slot.
    fn get_primary(&self) -> zbus::Result<String>;

    /// Represents the current (global) operation RAUC performs. Possible values are idle or installing.
    #[zbus(property)]
    fn operation(&self) -> zbus::Result<String>;

    /// Holds the last message of the last error that occurred.
    #[zbus(property)]
    fn last_error(&self) -> zbus::Result<String>;

    /// Provides installation progress information in the form
    /// (percentage, message, nesting depth)
    #[zbus(property)]
    fn progress(&self) -> zbus::Result<(i32, String, i32)>;

    /// Represents the system’s compatible. This can be used to check for usable bundles.
    #[zbus(property)]
    fn compatible(&self) -> zbus::Result<String>;

    /// Represents the system’s variant. This can be used to select parts of an bundle.
    #[zbus(property)]
    fn variant(&self) -> zbus::Result<String>;

    /// Contains the information RAUC uses to identify the booted slot. It is derived from the
    ///  kernel command line. This can either be the slot name (e.g. rauc.slot=rootfs.0) or the
    ///  root device path (e.g. root=PARTUUID=0815). If the root= kernel command line option is
    ///  used, the symlink is resolved to the block device (e.g. /dev/mmcblk0p1).
    #[zbus(property)]
    fn boot_slot(&self) -> zbus::Result<String>;

    /// This signal is emitted when an installation completed, either successfully or with an error.
    #[zbus(signal)]
    fn completed(&self, result: i32) -> Result<()>;
}

pub struct OTARauc<'a> {
    rauc: RaucProxy<'a>,
}

impl SystemUpdate for OTARauc<'static> {
    async fn install_bundle(&self, source: &str) -> eyre::Result<()> {
        self.rauc
            .install_bundle(source, std::collections::HashMap::new())
            .await?;
        Ok(())
    }

    async fn last_error(&self) -> eyre::Result<String> {
        self.rauc
            .last_error()
            .await
            .wrap_err("couldn't get dbus Rauc last error")
    }

    async fn info(&self, bundle: &str) -> eyre::Result<BundleInfo> {
        self.rauc
            .info(bundle)
            .await
            .wrap_err("couldn't get dbus Rauc info")
    }

    async fn operation(&self) -> eyre::Result<String> {
        self.rauc
            .operation()
            .await
            .wrap_err("couldn't get dbus Rauc operation")
    }

    async fn compatible(&self) -> eyre::Result<String> {
        self.rauc
            .compatible()
            .await
            .wrap_err("couldn't get dbus Rauc operation")
    }

    async fn boot_slot(&self) -> eyre::Result<String> {
        self.rauc
            .boot_slot()
            .await
            .wrap_err("couldn't get dbus Rauc boot slot")
    }

    async fn receive_completed(&self) -> eyre::Result<ProgressStream> {
        let progress_stream = self
            .rauc
            .receive_progress_changed()
            .await
            .then(|val| async move {
                let (percentage, message, _depth) = val.get().await?;

                Ok((percentage, message))
            })
            // Make the future fused, so we can select between the two streams
            .try_take_while(|(progress, _message)| future::ok(*progress < 100))
            .boxed();

        let completed_stream = self.rauc.receive_completed().await?;

        let stream = DeployStream::new(progress_stream, completed_stream);

        Ok(stream.boxed())
    }

    async fn get_primary(&self) -> eyre::Result<String> {
        self.rauc
            .get_primary()
            .await
            .wrap_err("couldn't get dbus Rauc primary partition")
    }

    async fn mark(&self, state: &str, slot_identifier: &str) -> eyre::Result<(String, String)> {
        self.rauc
            .mark(state, slot_identifier)
            .await
            .wrap_err("couldn't call dbus Rauc mark partition")
    }
}

impl<'a> OTARauc<'a> {
    #[instrument]
    pub async fn connect(config: &OtaConfig) -> eyre::Result<OTARauc<'a>> {
        let connection = match &config.rauc.dbus_socket {
            RaucDbus::System => zbus::Connection::system().await?,
            RaucDbus::Session => zbus::Connection::session().await?,
        };

        let rauc = RaucProxy::new(&connection).await?;

        info!(
            dbus_socket = %config.rauc.dbus_socket,
            "connected to RAUC dbus"
        );

        match rauc.boot_slot().await {
            Ok(boot_slot) => {
                info!(boot_slot);
            }
            Err(err) => {
                error!(
                    error = format!("{:#}", eyre::Report::new(err)),
                    "coultn't get boot slot"
                );
            }
        }

        match rauc.get_primary().await {
            Ok(primary_slot) => {
                info!(primary_slot);
            }
            Err(err) => {
                error!(
                    error = format!("{:#}", eyre::Report::new(err)),
                    "coultn't get the primary slot"
                );
            }
        }

        Ok(OTARauc { rauc })
    }
}

/// Progress of the Rauc deployment progress
struct DeployStream<S> {
    progress_changed: S,
    completed_stream: CompletedStream,
    completed: bool,
}

impl<S> DeployStream<S>
where
    S: Stream<Item = eyre::Result<(i32, String)>> + Unpin,
{
    fn new(progress_changed: S, completed_stream: CompletedStream) -> Self {
        Self {
            progress_changed,
            completed_stream,
            completed: false,
        }
    }

    fn map_completed(completed: Completed) -> eyre::Result<DeployStatus> {
        let signal = *completed.args()?.result();

        Ok(DeployStatus::Completed { signal })
    }

    fn poll_completed(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<eyre::Result<DeployStatus>>> {
        match ready!(self.completed_stream.poll_next_unpin(cx)) {
            Some(completed) => {
                debug!("deployment completed with signal: {:?}", completed);

                self.completed = true;

                Poll::Ready(Some(Self::map_completed(completed)))
            }
            None => {
                warn!("completed stream ended but poll_next was on the deploy progress");
                self.completed = true;

                Poll::Ready(None)
            }
        }
    }

    fn poll_progress(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<eyre::Result<DeployStatus>>> {
        match ready!(self.progress_changed.poll_next_unpin(cx)) {
            Some(Ok((percentage, message))) => {
                debug!("progress {message} {percentage}");

                Poll::Ready(Some(Ok(DeployStatus::Progress(DeployProgress {
                    percentage,
                    message,
                }))))
            }
            Some(Err(err)) => Poll::Ready(Some(Err(err))),
            None => {
                warn!("completed stream ended but poll_next was on the deploy progress");

                Poll::Ready(None)
            }
        }
    }
}

impl<S> Stream for DeployStream<S>
where
    S: Stream<Item = eyre::Result<(i32, String)>> + Unpin,
{
    type Item = eyre::Result<DeployStatus>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        if self.completed {
            return Poll::Ready(None);
        }

        match self.as_mut().poll_progress(cx) {
            Poll::Ready(Some(progress)) => {
                return Poll::Ready(Some(progress));
            }
            Poll::Ready(None) | Poll::Pending => {
                debug!("progress none or pending")
            }
        }

        match self.as_mut().poll_completed(cx) {
            Poll::Ready(completed) => Poll::Ready(completed),
            Poll::Pending => {
                debug!("completed is pending");
                Poll::Pending
            }
        }
    }
}

impl<S> FusedStream for DeployStream<S>
where
    S: Stream<Item = eyre::Result<(i32, String)>> + Unpin,
{
    fn is_terminated(&self) -> bool {
        self.completed
    }
}
