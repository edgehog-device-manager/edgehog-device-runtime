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

use serde::{Deserialize, Serialize};
use zbus::dbus_proxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

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
