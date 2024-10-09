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

use tracing::{debug, info, level_filters::LevelFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use zbus::{dbus_interface, ConnectionBuilder};

pub const SERVICE_NAME: &str = "io.edgehog.LedManager";

#[derive(Debug)]
struct LedManager {
    leds: Vec<String>,
}

#[dbus_interface(name = "io.edgehog.LedManager1")]
impl LedManager {
    fn list(&self) -> Vec<String> {
        debug!("listing {} leds", self.leds.len());

        self.leds.clone()
    }

    fn insert(&mut self, id: String) {
        debug!("adding led {id}");

        self.leds.push(id);
    }

    fn set(&self, id: String, status: bool) -> bool {
        const RESULT: bool = true;

        info!("SET {id} -> {status}: result {RESULT}");

        RESULT
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

    let leds = LedManager { leds: Vec::new() };

    let _conn = ConnectionBuilder::session()?
        .name(SERVICE_NAME)?
        .serve_at("/io/edgehog/LedManager", leds)?
        .build()
        .await?;

    info!("Service {SERVICE_NAME} started");

    tokio::signal::ctrl_c().await?;

    Ok(())
}
