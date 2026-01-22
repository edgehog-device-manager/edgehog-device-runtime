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

use astarte_device_sdk::{
    AstarteData, DeviceEvent, FromEvent, event::FromEventError, types::TypeError,
};
use tokio::time::{Duration, Instant, sleep};
use tracing::error;
use zbus::proxy;

use crate::controller::actor::Actor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedEvent {
    pub led_id: String,
    pub behavior: LedBehavior,
}

impl FromEvent for LedEvent {
    type Err = FromEventError;

    fn from_event(event: DeviceEvent) -> Result<Self, Self::Err> {
        let led_id =
            LedBehavior::led_id_from_path(&event.path).ok_or_else(|| FromEventError::Path {
                interface: "io.edgehog.devicemanager.LedBehavior",
                base_path: event.path.clone(),
            })?;

        LedBehavior::from_event(event).map(|behavior| LedEvent { led_id, behavior })
    }
}

#[derive(Debug, Clone, FromEvent, PartialEq, Eq)]
#[from_event(
    interface = "io.edgehog.devicemanager.LedBehavior",
    aggregation = "individual"
)]
pub enum LedBehavior {
    #[mapping(endpoint = "/%{led_id}/behavior")]
    Behavior(Blink),
}

impl LedBehavior {
    fn led_id_from_path(path: &str) -> Option<String> {
        path.strip_prefix('/')
            .and_then(|path| path.split_once('/').map(|(led_id, _)| led_id))
            .map(str::to_string)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blink {
    Single,
    Double,
    Slow,
}

impl TryFrom<AstarteData> for Blink {
    type Error = TypeError;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Blink60Seconds" => Ok(Self::Single),
            "DoubleBlink60Seconds" => Ok(Self::Double),
            "SlowBlink60Seconds" => Ok(Self::Slow),
            _ => {
                error!(value, "unrecognized LedBehavior behavior value");

                Err(TypeError::Conversion {
                    ctx: format!("unrecognized LedBehavior behavior value {value}"),
                })
            }
        }
    }
}

#[proxy(
    interface = "io.edgehog.LedManager1",
    default_service = "io.edgehog.LedManager",
    default_path = "/io/edgehog/LedManager"
)]
trait LedManager {
    async fn set(&self, id: String, status: bool) -> zbus::Result<bool>;
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
        use tracing::debug;

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

impl Actor for LedBlink {
    type Msg = LedEvent;

    fn task() -> &'static str {
        "led-behavior"
    }

    async fn init(&mut self) -> eyre::Result<()> {
        Ok(())
    }

    async fn handle(&mut self, msg: Self::Msg) -> eyre::Result<()> {
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
    use crate::controller::event::RuntimeEvent;

    use super::*;

    use astarte_device_sdk::Value;
    use astarte_device_sdk::chrono::Utc;
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

    #[test]
    fn should_convert_led_from_event() {
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.LedBehavior".to_string(),
            path: "/42/behavior".to_string(),
            data: Value::Individual {
                data: "Blink60Seconds".into(),
                timestamp: Utc::now(),
            },
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Led(LedEvent {
                led_id: "42".into(),
                behavior: LedBehavior::Behavior(Blink::Single)
            })
        );

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.LedBehavior".to_string(),
            path: "/42/behavior".to_string(),
            data: Value::Individual {
                data: "DoubleBlink60Seconds".into(),
                timestamp: Utc::now(),
            },
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Led(LedEvent {
                led_id: "42".into(),
                behavior: LedBehavior::Behavior(Blink::Double)
            })
        );

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.LedBehavior".to_string(),
            path: "/42/behavior".to_string(),
            data: Value::Individual {
                data: "SlowBlink60Seconds".into(),
                timestamp: Utc::now(),
            },
        };

        let res = RuntimeEvent::from_event(event).unwrap();

        assert_eq!(
            res,
            RuntimeEvent::Led(LedEvent {
                led_id: "42".into(),
                behavior: LedBehavior::Behavior(Blink::Slow)
            })
        );
    }
}
