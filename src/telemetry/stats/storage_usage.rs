// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
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

use astarte_device_sdk::IntoAstarteObject;
use astarte_device_sdk::chrono::Utc;
use sysinfo::{Disk, Disks};
use tracing::{error, instrument};

use crate::Client;
use crate::data::send_object_with_timestamp;
use crate::telemetry::sender::TelemetryTask;

pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.StorageUsage";

#[derive(Debug, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub struct DiskUsage {
    pub name: String,
    pub kind: String,
    pub fstype: String,
    pub mounts: Vec<String>,
    pub total_bytes: i64,
    pub free_bytes: i64,
}

impl DiskUsage {
    /// Get structured data for `io.edgehog.devicemanager.StorageUsage` interface.
    ///
    /// The `/dev/` prefix is excluded from the device names since it is common for all devices.
    pub async fn parse(disk: &Disk) -> Option<(String, Self)> {
        let Ok(total_bytes) = disk.total_space().try_into() else {
            error!("disk size too big, ignoring");
            return None;
        };

        let Ok(free_bytes) = disk.available_space().try_into() else {
            error!("available space too big, ignoring");
            return None;
        };

        let id = disk_uuid(disk).await?;

        Some((
            id,
            // Format to be send as aggregate object path
            DiskUsage {
                name: disk.name().display().to_string(),
                kind: disk.kind().to_string(),
                fstype: disk.file_system().display().to_string(),
                total_bytes,
                free_bytes,
                mounts: vec![disk.mount_point().display().to_string()],
            },
        ))
    }
}

#[derive(Debug, Default)]
pub(crate) struct StorageUsage {}

impl TelemetryTask for StorageUsage {
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send + Sync + 'static,
    {
        let timestamp = Utc::now();
        let disks = Disks::new_with_refreshed_list();

        let mut usage: Vec<(String, DiskUsage)> = Vec::new();

        for disk in disks.list() {
            if let Some((path, disk)) = DiskUsage::parse(disk).await {
                // Merge the mount points
                match usage.iter_mut().find(|(id, _)| *id == path) {
                    Some((_, entry)) => {
                        entry.mounts.extend(disk.mounts);
                    }
                    None => {
                        usage.push((path, disk));
                    }
                }
            }
        }

        for (id, disk) in usage {
            send_object_with_timestamp(client, INTERFACE, &id, disk, timestamp).await;
        }
    }
}

#[instrument(skip_all, fields(disk = %disk.name().display()))]
async fn disk_uuid(disk: &Disk) -> Option<String> {
    cfg_if::cfg_if! {
        if #[cfg(all(feature = "udev", target_os = "linux"))] {
            let dev = tokio::fs::metadata(disk.name())
                .await
                .inspect_err(|error| error!(%error, "couldn't get device number"))
                .ok()?;

            return Some(read_udev_uuid_prop(&dev));
        } else if #[cfg(windows)] {
            return Some(read_udev_uuid_prop(disk));
        } else if #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;

            let dev = tokio::fs::metadata(disk.name())
                .await
                .inspect_err(|error| error!(%error, "couldn't get device number"))
                .ok()?;

            // NOTE: use the rdev should be unique for device
            return Some(format!("/{}", dev.rdev()));
        } else {
            // NOTE: Use the digest of the file name, this is not stable and doesn't account for
            //       changes in the partitions, but should be unique for reboot and device.
            let val = hex::encode(
                aws_lc_rs::digest::digest(
                    &aws_lc_rs::digest::SHA512_256,
                    disk.name().as_encoded_bytes(),
                )
                    .as_ref(),
            );
            return Some(format!("/{}", val));
        }
    }
}

#[cfg(all(feature = "udev", target_os = "linux"))]
fn read_udev_uuid_prop(dev: &std::fs::Metadata) -> String {
    use std::os::linux::fs::MetadataExt;

    let opt_device = udev::Device::from_devnum(udev::DeviceType::Block, dev.st_rdev())
        .inspect_err(|error| error!(%error, "couldn't get device from devnum"))
        .ok();

    let Some(device) = opt_device else {
        return format!("/{}", dev.st_rdev());
    };

    let Some(id) = device.property_value("ID_FS_UUID_ENC") else {
        error!("couldn't get udev UUID property");

        return format!("/{}", dev.st_rdev());
    };

    format!("/{}", id.display())
}

#[cfg(target_os = "windows")]
fn read_udev_uuid_prop(dev: &Disk) -> String {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let mount: Vec<u16> = dev
        .mount_point()
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut serial_number = 0u32;

    let ret = unsafe {
        GetVolumeInformationW(
            mount.as_ptr(),
            std::ptr::null_mut(),
            0,
            &mut serial_number,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };

    if ret == 0 {
        error!(error = %std::io::Error::last_os_error(), "couldn't get volume serial");

        return format!("/{}", dev.name().display());
    }

    // Rust doesn't have built-in HIWORD/LOWORD macros,
    // but we can easily extract them using bitwise operations.
    let hi_word = serial_number >> 16;
    let lo_word = serial_number & 0xFFFF;

    format!("/{:04X}-{:04X}", hi_word, lo_word)
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;

    use super::*;

    #[test]
    fn should_serialize() {
        let data = DiskUsage {
            name: "/dev/sda1".to_string(),
            kind: "HDD".to_string(),
            fstype: "ext4".to_string(),
            mounts: vec!["/".to_string()],
            total_bytes: 50_000_000_000,
            free_bytes: 25_000_000_000,
        };

        let exp = AstarteObject::from_iter(
            [
                ("name", AstarteData::String("/dev/sda1".to_string())),
                ("kind", AstarteData::String("HDD".to_string())),
                ("fstype", AstarteData::String("ext4".to_string())),
                ("mounts", AstarteData::StringArray(vec!["/".to_string()])),
                ("totalBytes", AstarteData::LongInteger(data.total_bytes)),
                ("freeBytes", AstarteData::LongInteger(data.free_bytes)),
            ]
            .map(|(k, v)| (k.to_string(), v)),
        );

        let res = AstarteObject::try_from(data).unwrap();

        assert_eq!(res, exp);
    }

    #[tokio::test]
    async fn should_send_at_least_one() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        client
            .expect_send_object_with_timestamp()
            .times(1..)
            .with(
                predicate::eq("io.edgehog.devicemanager.StorageUsage"),
                predicate::always(),
                predicate::always(),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        StorageUsage {}.send(&mut client).await;
    }
}
