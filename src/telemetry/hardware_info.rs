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

use crate::data::{publish, Publisher};
use log::{debug, error};
use procfs::{CpuInfo, Meminfo, ProcResult};
use serde::Deserialize;

const INTERFACE: &str = "io.edgehog.devicemanager.HardwareInfo";

#[derive(Debug, Default, Deserialize)]
pub struct HardwareInfo {
    pub cpu: Cpu,
    pub mem: Mem,
}

impl HardwareInfo {
    pub async fn read() -> Self {
        let cpu = Cpu::read().await;
        let mem = Mem::read().await;

        HardwareInfo { cpu, mem }
    }

    /// get structured data for `io.edgehog.devicemanager.HardwareInfo` interface
    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        self.cpu.send(client).await;
        self.mem.send(client).await;
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
    pub architecture: String,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub vendor: Option<String>,
}

impl Cpu {
    async fn read() -> Self {
        let mut cpu = Cpu {
            architecture: get_machine_architecture(),
            ..Default::default()
        };
        match get_cpu_info() {
            Ok(mut cpu_info) => {
                cpu.model = cpu_info.fields.remove("model");
                cpu.model_name = cpu_info.fields.remove("model name");
                cpu.vendor = cpu_info.fields.remove("vendor_id");
            }
            Err(err) => {
                error!(
                    "couldn't get the cpu info: {}",
                    stable_eyre::Report::new(err)
                );
            }
        }

        cpu
    }

    async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        publish(client, INTERFACE, "/cpu/architecture", self.architecture).await;

        if let Some(model) = self.model {
            publish(client, INTERFACE, "/cpu/model", model).await;
        } else {
            debug!("missing cpu model");
        }

        if let Some(model_name) = self.model_name {
            publish(client, INTERFACE, "/cpu/modelName", model_name).await;
        } else {
            debug!("missing cpu model name");
        }

        if let Some(vendor_id) = self.vendor {
            publish(client, INTERFACE, "/cpu/vendor", vendor_id).await;
        } else {
            debug!("missing cpu vendor id");
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mem {
    pub total_bytes: Option<i64>,
}

impl Mem {
    async fn read() -> Self {
        let mut mem = Mem::default();

        let mem_info = match get_meminfo() {
            Ok(mem_info) => mem_info,
            Err(err) => {
                error!(
                    "couldn't get the memory info: {}",
                    stable_eyre::Report::new(err)
                );

                return mem;
            }
        };

        if let Ok(mem_total) = i64::try_from(mem_info.mem_total) {
            mem.total_bytes = Some(mem_total);
        } else {
            error!(
                "mem total too big to be sent to astarte: {}",
                mem_info.mem_total
            )
        }

        mem
    }

    async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        if let Some(total_bytes) = self.total_bytes {
            publish(client, INTERFACE, "/mem/totalBytes", total_bytes).await;
        } else {
            debug!("missing mem total bytes")
        }
    }
}

#[cfg(not(test))]
fn get_cpu_info() -> ProcResult<CpuInfo> {
    use procfs::Current;

    procfs::CpuInfo::current()
}

#[cfg(not(test))]
fn get_machine_architecture() -> String {
    std::env::consts::ARCH.to_owned()
}

#[cfg(not(test))]
fn get_meminfo() -> ProcResult<Meminfo> {
    use procfs::Current;

    procfs::Meminfo::current()
}

#[cfg(test)]
fn get_cpu_info() -> ProcResult<CpuInfo> {
    use procfs::FromRead;

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

    Ok(CpuInfo::from_read(r).unwrap())
}

#[cfg(test)]
fn get_machine_architecture() -> String {
    "test_architecture".to_owned()
}

#[cfg(test)]
fn get_meminfo() -> ProcResult<Meminfo> {
    use procfs::FromRead;

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
    Ok(Meminfo::from_read(r).unwrap())
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteType;
    use mockall::Sequence;

    use super::*;

    use crate::data::tests::MockPubSub;

    #[tokio::test]
    async fn hardware_info_test() {
        let mut client = MockPubSub::new();

        let mut seq = Sequence::new();

        client
            .expect_send()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.HardwareInfo"
                    && path == "/cpu/architecture"
                    && *data == AstarteType::String("test_architecture".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.HardwareInfo"
                    && path == "/cpu/model"
                    && *data == AstarteType::String("158".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.HardwareInfo"
                    && path == "/cpu/modelName"
                    && *data == AstarteType::String("ARMv7 Processor rev 10 (v7l)".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.HardwareInfo"
                    && path == "/cpu/vendor"
                    && *data == AstarteType::String("GenuineIntel".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_send()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.HardwareInfo"
                    && path == "/mem/totalBytes"
                    && *data == AstarteType::LongInteger(1043820544)
            })
            .returning(|_, _, _| Ok(()));

        HardwareInfo::read().await.send(&client).await;
    }
}
