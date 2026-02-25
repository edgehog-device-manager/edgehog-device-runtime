// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tracing::{debug, error};

use crate::Client;
use crate::data::set_property;

const INTERFACE: &str = "io.edgehog.devicemanager.HardwareInfo";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HardwareInfo {
    pub cpu: Cpu,
    pub mem: Mem,
}

impl HardwareInfo {
    pub async fn read() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing())
                .with_memory(MemoryRefreshKind::nothing().with_ram()),
        );

        let cpu = Cpu::read(&system).await;
        let mem = Mem::read(&system).await;

        HardwareInfo { cpu, mem }
    }

    /// get structured data for `io.edgehog.devicemanager.HardwareInfo` interface
    pub async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        self.cpu.send(client).await;
        self.mem.send(client).await;
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cpu {
    pub architecture: String,
    pub model: Option<String>,
    pub model_name: Option<String>,
    pub vendor: Option<String>,
}

impl Cpu {
    async fn read(system: &System) -> Self {
        let mut cpu = Cpu {
            architecture: std::env::consts::ARCH.into(),
            ..Default::default()
        };

        // The interface doesn't support multiple CPUs
        if let Some(info) = system.cpus().first() {
            cpu.model_name = Some(info.brand().to_string());
            cpu.vendor = Some(info.vendor_id().to_string());
        }

        #[cfg(any(target_os = "linux", target_os = "android"))]
        update_cpu_info(&mut cpu);

        cpu
    }

    async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        set_property(client, INTERFACE, "/cpu/architecture", self.architecture).await;

        if let Some(model) = self.model {
            set_property(client, INTERFACE, "/cpu/model", model).await;
        } else {
            debug!("missing cpu model");
        }

        if let Some(model_name) = self.model_name {
            set_property(client, INTERFACE, "/cpu/modelName", model_name).await;
        } else {
            debug!("missing cpu model name");
        }

        if let Some(vendor_id) = self.vendor {
            set_property(client, INTERFACE, "/cpu/vendor", vendor_id).await;
        } else {
            debug!("missing cpu vendor id");
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mem {
    pub total_bytes: Option<i64>,
}

impl Mem {
    async fn read(system: &System) -> Self {
        let mem_total = system.total_memory();

        let total_bytes = i64::try_from(mem_total)
            .inspect_err(|_| {
                error!(mem_total, "value too big to be sent to astarte",);
            })
            .ok();

        Mem { total_bytes }
    }

    async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        if let Some(total_bytes) = self.total_bytes {
            set_property(client, INTERFACE, "/mem/totalBytes", total_bytes).await;
        } else {
            debug!("missing mem total bytes")
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn update_cpu_info(cpu: &mut Cpu) {
    use procfs::{CpuInfo, Current, FromRead};

    let info = if cfg!(test) {
        let data = include_bytes!("../../../assets/procfs/cpu_info.txt");

        CpuInfo::from_read(data.as_slice())
    } else {
        CpuInfo::current()
    };

    let Some(mut info) = info
        .inspect_err(|error| error!(%error, "couldn't read CPU info"))
        .ok()
    else {
        return;
    };

    cpu.model = info.fields.remove("model");
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};

    use super::*;

    #[tokio::test]
    async fn hardware_info_test() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                predicate::eq("/cpu/architecture"),
                predicate::eq(AstarteData::String(std::env::consts::ARCH.to_string())),
            )
            .returning(|_, _, _| Ok(()));

        #[cfg(any(target_os = "linux", target_os = "android"))]
        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                predicate::eq("/cpu/model"),
                predicate::always(),
            )
            .returning(|_, _, _| Ok(()));

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                predicate::eq("/cpu/modelName"),
                predicate::always(),
            )
            .returning(|_, _, _| Ok(()));

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                predicate::eq("/cpu/vendor"),
                predicate::always(),
            )
            .returning(|_, _, _| Ok(()));

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.HardwareInfo"),
                predicate::eq("/mem/totalBytes"),
                predicate::always(),
            )
            .returning(|_, _, _| Ok(()));

        HardwareInfo::read().await.send(&mut client).await;
    }
}
