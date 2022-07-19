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

use crate::error::DeviceManagerError;
use astarte_sdk::types::AstarteType;
use procfs::{CpuInfo, Meminfo, ProcResult};
use std::collections::HashMap;

/// get structured data for `io.edgehog.devicemanager.HardwareInfo` interface
pub fn get_hardware_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    let architecture = get_machine_architecture();
    ret.insert("/cpu/architecture".to_owned(), architecture.into());

    let cpuinfo = get_cpu_info()?;
    if let Some(f) = cpuinfo.fields.get("model") {
        ret.insert("/cpu/model".to_owned(), f.clone().into());
    }

    if let Some(f) = cpuinfo.fields.get("model name") {
        ret.insert("/cpu/modelName".to_owned(), f.clone().into());
    }

    if let Some(f) = cpuinfo.fields.get("vendor_id") {
        ret.insert("/cpu/vendor".to_owned(), f.clone().into());
    }

    let meminfo = get_meminfo()?;
    ret.insert(
        "/mem/totalBytes".to_owned(),
        (meminfo.mem_total as i64).into(),
    );

    Ok(ret)
}

#[cfg(not(test))]
fn get_cpu_info() -> ProcResult<CpuInfo> {
    procfs::CpuInfo::new()
}

#[cfg(not(test))]
fn get_machine_architecture() -> String {
    std::env::consts::ARCH.to_owned()
}

#[cfg(not(test))]
fn get_meminfo() -> ProcResult<Meminfo> {
    procfs::Meminfo::new()
}

#[cfg(test)]
fn get_cpu_info() -> ProcResult<CpuInfo> {
    let data = r#"processor       : 0
vendor_id       : GenuineIntel
model           : 158
model name      : ARMv7 Processor rev 10 (v7l)
BogoMIPS        : 6.00
Features        : half thumb fastmult vfp edsp neon vfpv3 tls vfpd32
CPU implementer : 0x41
CPU architecture: 7
CPU variant     : 0x2
CPU part        : 0xc09
CPU revision    : 10

Hardware        : Freescale i.MX6 SoloX (Device Tree)
Revision        : 0000
Serial          : 0000000000000000
"#;

    let r = std::io::Cursor::new(data.as_bytes());

    Ok(CpuInfo::from_reader(r).unwrap())
}

#[cfg(test)]
fn get_machine_architecture() -> String {
    "test_architecture".to_owned()
}

#[cfg(test)]
fn get_meminfo() -> ProcResult<Meminfo> {
    let data = r#"MemTotal:        1019356 kB
MemFree:          739592 kB
MemAvailable:     802296 kB
Buffers:            7372 kB
Cached:            88364 kB
SwapCached:            0 kB
Active:            41328 kB
Inactive:          64908 kB
Active(anon):       1224 kB
Inactive(anon):    35160 kB
Active(file):      40104 kB
Inactive(file):    29748 kB
Unevictable:           0 kB
Mlocked:               0 kB
HighTotal:             0 kB
HighFree:              0 kB
LowTotal:        1019356 kB
LowFree:          739592 kB
SwapTotal:             0 kB
SwapFree:              0 kB
Dirty:                 4 kB
Writeback:             0 kB
AnonPages:         10500 kB
Mapped:            21688 kB
Shmem:             25884 kB
KReclaimable:       8452 kB
Slab:              21180 kB
SReclaimable:       8452 kB
SUnreclaim:        12728 kB
KernelStack:         752 kB
PageTables:          656 kB
NFS_Unstable:          0 kB
Bounce:                0 kB
WritebackTmp:          0 kB
CommitLimit:      509676 kB
Committed_AS:     139696 kB
VmallocTotal:    1032192 kB
VmallocUsed:        6648 kB
VmallocChunk:          0 kB
Percpu:              376 kB
CmaTotal:         327680 kB
CmaFree:          194196 kB
"#;

    let r = std::io::Cursor::new(data.as_bytes());
    Ok(Meminfo::from_reader(r).unwrap())
}

#[cfg(test)]
mod tests {
    use crate::telemetry::hardware_info::get_hardware_info;
    use astarte_sdk::types::AstarteType;

    #[test]
    fn hardware_info_test() {
        let astarte_hardware_info = get_hardware_info().unwrap();
        assert_eq!(
            astarte_hardware_info
                .get("/cpu/architecture")
                .unwrap()
                .to_owned(),
            AstarteType::String("test_architecture".to_string())
        );
        assert_eq!(
            astarte_hardware_info.get("/cpu/model").unwrap().to_owned(),
            AstarteType::String("158".to_string())
        );
        assert_eq!(
            astarte_hardware_info
                .get("/cpu/modelName")
                .unwrap()
                .to_owned(),
            AstarteType::String("ARMv7 Processor rev 10 (v7l)".to_string())
        );
        assert_eq!(
            astarte_hardware_info.get("/cpu/vendor").unwrap().to_owned(),
            AstarteType::String("GenuineIntel".to_string())
        );
        assert_eq!(
            astarte_hardware_info
                .get("/mem/totalBytes")
                .unwrap()
                .to_owned(),
            AstarteType::LongInteger(1043820544)
        );
    }
}
