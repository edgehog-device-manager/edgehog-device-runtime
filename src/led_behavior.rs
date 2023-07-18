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

use tokio::time::{sleep, Duration, Instant};
use zbus::dbus_proxy;

#[dbus_proxy(
    interface = "io.edgehog.LedManager1",
    default_service = "io.edgehog.LedManager",
    default_path = "/io/edgehog/LedManager"
)]
trait LedManager {
    fn set(&self, id: String, status: bool) -> zbus::Result<bool>;
}

struct BlinkConf {
    repetitions: u64,
    end_time_secs: u64,
    after_on_delay_millis: u64,
    after_off_delay_millis: u64,
    end_cycle_delay_millis: u64,
}

pub(crate) async fn set_behavior(led_id: String, behavior: String) -> bool {
    let set_behavior = match &behavior[..] {
        "Blink60Seconds" => blink_60_seconds(led_id).await,
        "DoubleBlink60Seconds" => double_blink_60_seconds(led_id).await,
        "SlowBlink60Seconds" => slow_blink_60_seconds(led_id).await,
        _ => {
            log::error!("Unexpected behavior");
            return false;
        }
    };

    set_behavior.unwrap_or(false)
}

async fn blink_60_seconds(led_id: String) -> zbus::Result<bool> {
    let conf = BlinkConf {
        end_time_secs: 60,
        repetitions: 1,
        after_on_delay_millis: 1000,
        after_off_delay_millis: 1000,
        end_cycle_delay_millis: 0,
    };
    blink(led_id, conf).await
}

async fn double_blink_60_seconds(led_id: String) -> zbus::Result<bool> {
    let conf = BlinkConf {
        repetitions: 2,
        end_time_secs: 60,
        after_on_delay_millis: 300,
        after_off_delay_millis: 200,
        end_cycle_delay_millis: 800,
    };
    blink(led_id, conf).await
}

async fn slow_blink_60_seconds(led_id: String) -> zbus::Result<bool> {
    let conf = BlinkConf {
        end_time_secs: 60,
        repetitions: 1,
        after_on_delay_millis: 2000,
        after_off_delay_millis: 2000,
        end_cycle_delay_millis: 0,
    };
    blink(led_id, conf).await
}

#[cfg(not(test))]
async fn blink(led_id: String, conf: BlinkConf) -> zbus::Result<bool> {
    let connection = zbus::Connection::system().await?;
    let led_manager = LedManagerProxy::new(&connection).await?;

    let start = Instant::now();
    while (Instant::now() - start).as_secs() < conf.end_time_secs {
        for _i in 0..conf.repetitions {
            println!("ON");
            if !led_manager.set(led_id.clone(), true).await? {
                return Ok(false);
            }
            sleep(Duration::from_millis(conf.after_on_delay_millis)).await;
            println!("OFF");
            if !led_manager.set(led_id.clone(), false).await? {
                return Ok(false);
            }
            sleep(Duration::from_millis(conf.after_off_delay_millis)).await;
        }
        sleep(Duration::from_millis(conf.end_cycle_delay_millis)).await;
    }
    Ok(true)
}

#[cfg(test)]
async fn blink(_led_id: String, conf: BlinkConf) -> zbus::Result<bool> {
    use std::io::Write;
    let mut out = std::io::stdout();
    let start = Instant::now();
    while (Instant::now() - start).as_secs() < conf.end_time_secs / 10 {
        for _i in 0..conf.repetitions {
            print!("█");
            out.flush().unwrap();
            sleep(Duration::from_millis(conf.after_on_delay_millis)).await;
            print!("\r \r");
            out.flush().unwrap();
            sleep(Duration::from_millis(conf.after_off_delay_millis)).await;
        }
        sleep(Duration::from_millis(conf.end_cycle_delay_millis)).await;
    }
    println!();
    Ok(true)
}

#[cfg(test)]
mod tests {
    use crate::led_behavior::set_behavior;
    use tokio::time::Duration;

    #[tokio::test]
    async fn set_behavior_test() {
        let steps = [
            "Blink60Seconds".to_string(),
            "DoubleBlink60Seconds".to_string(),
            "SlowBlink60Seconds".to_string(),
        ];

        tokio::time::pause();

        for step in steps {
            let handler = tokio::spawn(async move { set_behavior("".to_string(), step).await });

            tokio::time::advance(Duration::from_secs(42)).await;

            assert!(handler.await.expect("join error"));
        }

        assert!(!set_behavior("".to_string(), "Blink30Seconds".to_string()).await);
    }
}
