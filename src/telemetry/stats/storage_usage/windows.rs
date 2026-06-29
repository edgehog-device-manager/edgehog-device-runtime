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

/// Returns the disk serial number or the disk name on error.
pub(super) fn read_disk_serial_number(dev: &Disk) -> String {
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
