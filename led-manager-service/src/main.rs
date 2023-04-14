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

use zbus::{dbus_interface, ConnectionBuilder};

struct LedManager {
    leds: Vec<String>,
}

#[dbus_interface(name = "io.edgehog.LedManager1")]
impl LedManager {
    fn list(&self) -> Vec<String> {
        self.leds.clone()
    }

    fn insert(&mut self, id: String) {
        self.leds.push(id);
    }

    fn set(&self, id: String, status: bool) -> bool {
        let result = true;
        print!("SET {} -> {}: result {}", id, status, result);
        result
    }
}

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let leds = LedManager { leds: Vec::new() };

    ConnectionBuilder::session()?
        .name("io.edgehog.LedManager")?
        .serve_at("/io/edgehog/LedManager", leds)?
        .build()
        .await?;

    loop {
        std::thread::park()
    }
}
