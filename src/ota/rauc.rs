/*
 * This file is part of Edgehog.
 *
 * Copyright 2022 SECO Mind Srl
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *   http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 *
 * SPDX-License-Identifier: Apache-2.0
 */

use std::task::Poll;

use async_trait::async_trait;
use futures::stream::FusedStream;
use futures::{future, ready, Stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use zbus::proxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

use crate::error::DeviceManagerError;
use crate::ota::{DeployProgress, DeployStatus, SystemUpdate};

use super::ProgressStream;

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
trait Rauc {
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

#[async_trait]
impl SystemUpdate for OTARauc<'static> {
    async fn install_bundle(&self, source: &str) -> Result<(), DeviceManagerError> {
        self.rauc
            .install_bundle(source, std::collections::HashMap::new())
            .await?;
        Ok(())
    }

    async fn last_error(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .last_error()
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn info(&self, bundle: &str) -> Result<BundleInfo, DeviceManagerError> {
        self.rauc
            .info(bundle)
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn operation(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .operation()
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn compatible(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .compatible()
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn boot_slot(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .boot_slot()
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn receive_completed(&self) -> Result<ProgressStream, DeviceManagerError> {
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

    async fn get_primary(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .get_primary()
            .await
            .map_err(DeviceManagerError::Zbus)
    }

    async fn mark(
        &self,
        state: &str,
        slot_identifier: &str,
    ) -> Result<(String, String), DeviceManagerError> {
        self.rauc
            .mark(state, slot_identifier)
            .await
            .map_err(DeviceManagerError::Zbus)
    }
}

impl<'a> OTARauc<'a> {
    pub async fn connect() -> Result<OTARauc<'a>, DeviceManagerError> {
        let connection = zbus::Connection::system().await?;

        let proxy = RaucProxy::new(&connection).await?;

        info!("boot slot = {:?}", proxy.boot_slot().await);
        info!("primary slot = {:?}", proxy.get_primary().await);

        Ok(OTARauc { rauc: proxy })
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
    S: Stream<Item = Result<(i32, String), DeviceManagerError>> + Unpin,
{
    fn new(progress_changed: S, completed_stream: CompletedStream) -> Self {
        Self {
            progress_changed,
            completed_stream,
            completed: false,
        }
    }

    fn map_completed(completed: Completed) -> Result<DeployStatus, DeviceManagerError> {
        let signal = *completed.args()?.result();

        Ok(DeployStatus::Completed { signal })
    }

    fn poll_completed(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Result<DeployStatus, DeviceManagerError>>> {
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
    ) -> Poll<Option<Result<DeployStatus, DeviceManagerError>>> {
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
    S: Stream<Item = Result<(i32, String), DeviceManagerError>> + Unpin,
{
    type Item = Result<DeployStatus, DeviceManagerError>;

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
    S: Stream<Item = Result<(i32, String), DeviceManagerError>> + Unpin,
{
    fn is_terminated(&self) -> bool {
        self.completed
    }
}
