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

use async_trait::async_trait;
use tokio::time::{sleep, Duration, Instant};
use zbus::dbus_proxy;

use crate::controller::{
    actor::Actor,
    message::{Blink, LedBehavior, LedEvent},
};

#[dbus_proxy(
    interface = "io.edgehog.LedManager1",
    default_service = "io.edgehog.LedManager",
    default_path = "/io/edgehog/LedManager"
)]
trait LedManager {
    fn set(&self, id: String, status: bool) -> zbus::Result<bool>;
}

#[derive(Debug, Clone, Copy)]
struct BlinkConf {
    repetitions: u64,
    end_time_secs: u64,
    after_on_delay_millis: u64,
    after_off_delay_millis: u64,
    end_cycle_delay_millis: u64,
}

impl From<Blink> for BlinkConf {
    fn from(value: Blink) -> Self {
        match value {
            Blink::Single => BlinkConf {
                end_time_secs: 60,
                repetitions: 1,
                after_on_delay_millis: 1000,
                after_off_delay_millis: 1000,
                end_cycle_delay_millis: 0,
            },
            Blink::Double => BlinkConf {
                repetitions: 2,
                end_time_secs: 60,
                after_on_delay_millis: 300,
                after_off_delay_millis: 200,
                end_cycle_delay_millis: 800,
            },
            Blink::Slow => BlinkConf {
                end_time_secs: 60,
                repetitions: 1,
                after_on_delay_millis: 2000,
                after_off_delay_millis: 2000,
                end_cycle_delay_millis: 0,
            },
        }
    }
}

impl BlinkConf {
    #[cfg(not(test))]
    async fn blink(self, led_id: String) -> zbus::Result<()> {
        use log::debug;

        let connection = zbus::Connection::system().await?;
        let led_manager = LedManagerProxy::new(&connection).await?;

        let start = Instant::now();
        while (Instant::now() - start).as_secs() < self.end_time_secs {
            for _i in 0..self.repetitions {
                debug!("Turning led on");
                if !led_manager.set(led_id.clone(), true).await? {
                    return Ok(());
                }
                sleep(Duration::from_millis(self.after_on_delay_millis)).await;
                debug!("Turning led off");
                if !led_manager.set(led_id.clone(), false).await? {
                    return Ok(());
                }
                sleep(Duration::from_millis(self.after_off_delay_millis)).await;
            }
            sleep(Duration::from_millis(self.end_cycle_delay_millis)).await;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn blink(self, _led_id: String) -> zbus::Result<()> {
        let start = Instant::now();
        while (Instant::now() - start).as_secs() < self.end_time_secs / 10 {
            for _i in 0..self.repetitions {
                print!("█");
                sleep(Duration::from_millis(self.after_on_delay_millis)).await;
                print!("\r \r");
                sleep(Duration::from_millis(self.after_off_delay_millis)).await;
            }
            sleep(Duration::from_millis(self.end_cycle_delay_millis)).await;
        }
        println!();

        Ok(())
    }
}

pub struct LedBlink;

#[async_trait]
impl Actor for LedBlink {
    type Msg = LedEvent;

    fn task() -> &'static str {
        "led-behavior"
    }

    async fn init(&mut self) -> stable_eyre::Result<()> {
        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> stable_eyre::Result<()> {
        match msg.behavior {
            LedBehavior::Behavior(blink) => {
                BlinkConf::from(blink).blink(msg.led_id).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::time::Duration;

    #[tokio::test]
    async fn set_behavior_test() {
        let steps = [Blink::Single, Blink::Double, Blink::Slow];

        let mut led = LedBlink;

        tokio::time::pause();
        for step in steps {
            tokio::time::advance(Duration::from_secs(42)).await;

            led.handle(LedEvent {
                led_id: "led_1".to_string(),
                behavior: LedBehavior::Behavior(step),
            })
            .await
            .unwrap()
        }

        tokio::time::resume();
    }
}
