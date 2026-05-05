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

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::MAX_PATH;
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDiskFreeSpaceW, GetVolumePathNameW,
};

use crate::file_transfer::file_system::store::FsStat;

pub(crate) fn get_disk_free_space(path: &Path) -> io::Result<FsStat> {
    // https://learn.microsoft.com/en-us/windows/win32/fileio/maximum-file-path-limitation
    // Should be 260
    const BUFF_SIZE: u32 = MAX_PATH + 1;

    let mut vol = [0u16; BUFF_SIZE as usize];
    let path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let ret = unsafe { GetVolumePathNameW(path.as_ptr(), vol.as_mut_ptr(), BUFF_SIZE) };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut user_avail = 0u64;

    let ret = unsafe {
        GetDiskFreeSpaceExW(
            vol.as_ptr(),
            &mut user_avail,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut sectors_per_cluster = 0;
    let mut bytes_per_sector = 0;
    let mut total_number_of_clusters = 0;

    let ret = unsafe {
        GetDiskFreeSpaceW(
            vol.as_ptr(),
            &mut sectors_per_cluster,
            &mut bytes_per_sector,
            std::ptr::null_mut(),
            &mut total_number_of_clusters,
        )
    };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    let bytes_per_cluster =
        u64::from(sectors_per_cluster).saturating_mul(u64::from(bytes_per_sector));
    let fs_total = bytes_per_cluster.saturating_mul(u64::from(total_number_of_clusters));

    Ok(FsStat {
        fragment_size: bytes_per_cluster,
        user_avail,
        fs_total,
    })
}
