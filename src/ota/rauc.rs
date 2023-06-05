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

use async_trait::async_trait;
use log::info;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use zbus::dbus_proxy;
use zbus::export::futures_util::StreamExt;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

use crate::ota::{DeployingProgress, OtaError, SystemUpdate};
use crate::DeviceManagerError;

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

#[dbus_proxy(
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

    // properties

    /// Represents the current (global) operation RAUC performs. Possible values are idle or installing.
    #[dbus_proxy(property)]
    fn operation(&self) -> zbus::Result<String>;

    /// Holds the last message of the last error that occurred.
    #[dbus_proxy(property)]
    fn last_error(&self) -> zbus::Result<String>;

    /// Provides installation progress information in the form
    /// (percentage, message, nesting depth)
    #[dbus_proxy(property)]
    fn progress(&self) -> zbus::Result<(i32, String, i32)>;

    /// Represents the system’s compatible. This can be used to check for usable bundles.
    #[dbus_proxy(property)]
    fn compatible(&self) -> zbus::Result<String>;

    /// Represents the system’s variant. This can be used to select parts of an bundle.
    #[dbus_proxy(property)]
    fn variant(&self) -> zbus::Result<String>;

    /// Contains the information RAUC uses to identify the booted slot. It is derived from the
    ///  kernel command line. This can either be the slot name (e.g. rauc.slot=rootfs.0) or the
    ///  root device path (e.g. root=PARTUUID=0815). If the root= kernel command line option is
    ///  used, the symlink is resolved to the block device (e.g. /dev/mmcblk0p1).
    #[dbus_proxy(property)]
    fn boot_slot(&self) -> zbus::Result<String>;

    // signal

    /// This signal is emitted when an installation completed, either successfully or with an error.
    #[dbus_proxy(signal)]
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
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn info(&self, bundle: &str) -> Result<BundleInfo, DeviceManagerError> {
        self.rauc
            .info(bundle)
            .await
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn operation(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .operation()
            .await
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn compatible(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .compatible()
            .await
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn boot_slot(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .boot_slot()
            .await
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn receive_completed(
        &self,
        sender: Sender<DeployingProgress>,
    ) -> Result<i32, DeviceManagerError> {
        let stream_progress_fn = async move {
            let mut progress_stream = self.rauc.receive_progress_changed().await;

            while let Some(value) = progress_stream.next().await {
                if let Ok((percentage, message, _)) = value.get().await {
                    if sender
                        .send(DeployingProgress {
                            percentage,
                            message,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        };

        let mut receive_completed_update = self.rauc.receive_completed().await?;
        let completed_update = tokio::select! {
            _ = stream_progress_fn => {None}
            completed_update = receive_completed_update.next() => {completed_update}
        };

        if let Some(completed) = completed_update {
            let signal = completed.args().unwrap();
            let signal = *signal.result();

            Ok(signal)
        } else {
            Err(DeviceManagerError::OtaError(OtaError::Internal(
                "Unable to receive signal from rauc interface",
            )))
        }
    }

    async fn get_primary(&self) -> Result<String, DeviceManagerError> {
        self.rauc
            .get_primary()
            .await
            .map_err(DeviceManagerError::ZbusError)
    }

    async fn mark(
        &self,
        state: &str,
        slot_identifier: &str,
    ) -> Result<(String, String), DeviceManagerError> {
        self.rauc
            .mark(state, slot_identifier)
            .await
            .map_err(DeviceManagerError::ZbusError)
    }
}

impl<'a> OTARauc<'a> {
    pub async fn new() -> Result<OTARauc<'a>, DeviceManagerError> {
        let connection = zbus::Connection::system().await?;

        let proxy = RaucProxy::new(&connection).await?;

        info!("boot slot = {:?}", proxy.boot_slot().await);
        info!("primary slot = {:?}", proxy.get_primary().await);

        Ok(OTARauc { rauc: proxy })
    }
}
