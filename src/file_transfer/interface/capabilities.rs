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

use astarte_device_sdk::Client;
use tracing::{info, instrument};

use crate::data::set_property;

/// File transfer capabilities the Device supports
pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.fileTransfer.Capabilities";
/// Encoding for tar.gz
pub(crate) const TAR_GZ: &str = "tar.gz";
/// Encoding for gz
pub(crate) const GZ: &str = "gz";
/// Storage target
pub(crate) const STORAGE_TARGET: &str = "storage";
/// Streaming target
pub(crate) const STREAMING_TARGET: &str = "streaming";
/// Filesystem target
pub(crate) const FILESYSTEM_TARGET: &str = "filesystem";
/// Capabilities of the device
pub(crate) const CAPABILITIES: Capabilities<3> = Capabilities {
    unix_permissions: true,
    upload: [
        TargetCapability {
            target: STORAGE_TARGET,
            encodings: &[TAR_GZ, GZ],
        },
        TargetCapability {
            target: STREAMING_TARGET,
            encodings: &[],
        },
        TargetCapability {
            target: FILESYSTEM_TARGET,
            encodings: &[TAR_GZ, GZ],
        },
    ],
    download: [
        TargetCapability {
            target: STORAGE_TARGET,
            encodings: &[TAR_GZ, GZ],
        },
        TargetCapability {
            target: STREAMING_TARGET,
            encodings: &[GZ],
        },
        TargetCapability {
            target: FILESYSTEM_TARGET,
            encodings: &[TAR_GZ, GZ],
        },
    ],
};

#[derive(Debug, Clone)]
pub(crate) struct Capabilities<const T: usize> {
    unix_permissions: bool,
    upload: [TargetCapability; T],
    download: [TargetCapability; T],
}

#[derive(Debug, Clone)]
pub(crate) struct TargetCapability {
    target: &'static str,
    encodings: &'static [&'static str],
}

impl<const T: usize> Capabilities<T> {
    const DEVICE_TO_SERVER: &str = "deviceToServer";
    const SERVER_TO_DEVICE: &str = "serverToDevice";

    #[instrument(skip(device))]
    pub(crate) async fn send<D>(&self, device: &mut D)
    where
        D: Client,
    {
        set_property(
            device,
            INTERFACE,
            "/transfer/unixPermissions",
            self.unix_permissions,
        )
        .await;

        Self::send_type_capabilities(Self::DEVICE_TO_SERVER, &self.upload, device).await;
        Self::send_type_capabilities(Self::SERVER_TO_DEVICE, &self.download, device).await;

        info!("device capabilities set");
    }

    async fn send_type_capabilities<D>(
        transfer_type: &str,
        target_capabilities: &[TargetCapability; T],
        device: &mut D,
    ) where
        D: Client,
    {
        let targets = target_capabilities
            .iter()
            .map(|c| c.target.to_string())
            .collect::<Vec<String>>();

        set_property(
            device,
            INTERFACE,
            &format!("/transfer/{transfer_type}/targets"),
            targets,
        )
        .await;

        for target_capab in target_capabilities {
            let target = target_capab.target;
            let encodings: Vec<String> = target_capab
                .encodings
                .iter()
                .map(|e| e.to_string())
                .collect();

            set_property(
                device,
                INTERFACE,
                &format!("/{transfer_type}/{target}/encodings"),
                encodings,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::{store::SqliteStore, transport::Connection};
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;

    use super::*;

    fn expect_target<C: Connection>(device: &mut MockDeviceClient<C>, direction: &str) {
        device
            .expect_set_property()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq(format!("/transfer/{direction}/targets")),
                predicate::eq(AstarteData::StringArray(
                    ["storage", "streaming", "filesystem"]
                        .map(str::to_string)
                        .to_vec(),
                )),
            )
            .returning(|_, _, _| Ok(()));
    }

    fn expect_encoding<C: Connection>(
        device: &mut MockDeviceClient<C>,
        direction: &str,
        target: &str,
        mut encodings: Vec<String>,
    ) {
        encodings.sort_unstable();

        device
            .expect_set_property()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq(format!("/{direction}/{target}/encodings")),
                predicate::function(move |d| {
                    let mut cloned = match d {
                        AstarteData::StringArray(items) => items.clone(),
                        _ => panic!("unexpected type"),
                    };

                    cloned.sort_unstable();

                    cloned == encodings
                }),
            )
            .returning(|_, _, _| Ok(()));
    }

    #[tokio::test]
    async fn set_capabilities() {
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        device
            .expect_set_property()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.fileTransfer.Capabilities"),
                predicate::eq("/transfer/unixPermissions"),
                predicate::eq(AstarteData::Boolean(true)),
            )
            .returning(|_, _, _| Ok(()));

        expect_target(&mut device, "serverToDevice");
        expect_encoding(
            &mut device,
            "serverToDevice",
            "filesystem",
            ["tar.gz", "gz"].map(str::to_string).to_vec(),
        );
        expect_encoding(
            &mut device,
            "serverToDevice",
            "streaming",
            ["gz"].map(str::to_string).to_vec(),
        );
        expect_encoding(
            &mut device,
            "serverToDevice",
            "storage",
            ["tar.gz", "gz"].map(str::to_string).to_vec(),
        );

        expect_target(&mut device, "deviceToServer");
        expect_encoding(
            &mut device,
            "deviceToServer",
            "filesystem",
            ["tar.gz", "gz"].map(str::to_string).to_vec(),
        );
        expect_encoding(
            &mut device,
            "deviceToServer",
            "streaming",
            [].map(str::to_string).to_vec(),
        );
        expect_encoding(
            &mut device,
            "deviceToServer",
            "storage",
            ["tar.gz", "gz"].map(str::to_string).to_vec(),
        );

        CAPABILITIES.send(&mut device).await;
    }
}
