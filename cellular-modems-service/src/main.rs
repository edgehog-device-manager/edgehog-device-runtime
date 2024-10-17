// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use tracing::{debug, info, level_filters::LevelFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use zbus::{
    dbus_interface,
    zvariant::{DeserializeDict, SerializeDict, Type},
    ConnectionBuilder,
};

pub const SERVICE_NAME: &str = "io.edgehog.CellularModems1";

#[derive(Debug, Default, Clone, DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
struct ModemProperties {
    apn: String,
    imei: String,
    imsi: String,
}

struct CellularModems {
    modems: HashMap<String, ModemProperties>,
}

#[dbus_interface(name = "io.edgehog.CellularModems1")]
impl CellularModems {
    fn list(&self) -> Vec<String> {
        self.modems.keys().cloned().collect()
    }

    fn get(&self, id: String) -> ModemProperties {
        self.modems.get(&id).cloned().unwrap_or_else(|| {
            debug!("modem {id} not found");

            ModemProperties::default()
        })
    }

    fn insert(&mut self, id: String, apn: String, imei: String, imsi: String) {
        info!("inserting modem {id}");

        self.modems.insert(id, ModemProperties { apn, imei, imsi });
    }
}

#[tokio::main]
async fn main() -> stable_eyre::Result<()> {
    stable_eyre::install()?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init()?;

    let cellular_modems = CellularModems {
        modems: HashMap::new(),
    };

    let _conn = ConnectionBuilder::session()?
        .name(SERVICE_NAME)?
        .serve_at("/io/edgehog/CellularModems", cellular_modems)?
        .build()
        .await?;

    info!("Service {SERVICE_NAME} started");

    tokio::signal::ctrl_c().await?;

    Ok(())
}
