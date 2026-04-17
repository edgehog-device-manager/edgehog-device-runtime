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

//! Capabilities for the Device.

use astarte_device_sdk::{AstarteData, Client};
use tracing::{info, instrument};

use crate::data::set_property;

/// File transfer capabilities the Device supports
pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.fileTransfer.Capabilities";
/// Encoding for tar.gz
pub(crate) const TAR_GZ: &str = "tar.gz";
/// Storage target
pub(crate) const STORAGE_TARGET: &str = "storage";
/// Streaming target
pub(crate) const STREAMING_TARGET: &str = "streaming";
/// Filesystem target
pub(crate) const FILESYSTEM_TARGET: &str = "filesystem";
/// Capabilities of the device
pub(crate) const CAPABILITIES: Capabilities<1, 3> = Capabilities {
    encodings: [TAR_GZ],
    unix_permissions: true,
    targets: [STORAGE_TARGET, STREAMING_TARGET, FILESYSTEM_TARGET],
};

#[derive(Debug)]
pub(crate) struct Capabilities<const E: usize, const T: usize> {
    encodings: [&'static str; E],
    unix_permissions: bool,
    targets: [&'static str; T],
}

impl<const E: usize, const T: usize> Capabilities<E, T> {
    #[instrument(skip(device))]
    pub(crate) async fn send<D>(&self, device: &mut D)
    where
        D: Client,
    {
        let encodings = AstarteData::StringArray(self.encodings.map(str::to_string).to_vec());
        let targets = AstarteData::StringArray(self.targets.map(str::to_string).to_vec());

        set_property(device, INTERFACE, "/encodings", encodings).await;
        set_property(device, INTERFACE, "/unixPermissions", self.unix_permissions).await;
        set_property(device, INTERFACE, "/targets", targets).await;

        info!("device capabilities set");
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};

    use super::*;

    #[tokio::test]
    async fn set_capabilities() {
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq("/encodings"),
                predicate::eq(AstarteData::StringArray(
                    ["tar.gz"].map(str::to_string).to_vec(),
                )),
            )
            .returning(|_, _, _| Ok(()));
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq("/unixPermissions"),
                predicate::eq(AstarteData::Boolean(true)),
            )
            .returning(|_, _, _| Ok(()));
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq("/targets"),
                predicate::eq(AstarteData::StringArray(
                    ["storage", "streaming", "filesystem"]
                        .map(str::to_string)
                        .to_vec(),
                )),
            )
            .returning(|_, _, _| Ok(()));

        CAPABILITIES.send(&mut device).await;
    }
}
