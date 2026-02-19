// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Cellular connection properties telemetry information.

use eyre::WrapErr;
use futures::StreamExt;
use std::collections::HashMap;
use tracing::{debug, error};
use zbus::proxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

use crate::Client;
use crate::data::set_property;

const INTERFACE: &str = "io.edgehog.devicemanager.CellularConnectionProperties";

#[derive(Debug, Clone, DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct ModemProperties {
    apn: String,
    imei: String,
    imsi: String,
}

#[proxy(
    interface = "io.edgehog.CellularModems1",
    default_service = "io.edgehog.CellularModems",
    default_path = "/io/edgehog/CellularModems"
)]
trait CellularModems {
    fn list(&self) -> zbus::Result<Vec<String>>;
    fn get(&self, id: String) -> zbus::Result<ModemProperties>;
}

#[derive(Debug, Clone, Default)]
pub struct CellularConnection {
    properties: HashMap<String, ModemProperties>,
}

impl CellularConnection {
    pub async fn read() -> CellularConnection {
        match Self::get_cellular_properties().await {
            Ok(properties) => CellularConnection { properties },
            Err(err) => {
                error!("{err}");

                CellularConnection::default()
            }
        }
    }

    async fn get_cellular_properties() -> eyre::Result<HashMap<String, ModemProperties>> {
        let connection = zbus::Connection::session().await?;
        let proxy = CellularModemsProxy::new(&connection).await?;

        let modems = proxy.list().await?;

        let properties = futures::stream::iter(modems)
            .then(|id| async {
                proxy
                    .get(id.clone())
                    .await
                    .wrap_err_with(|| format!("couldn't get modem {id}"))
                    .map(|modem| (id, modem))
            })
            .filter_map(|res| async {
                let (id, modem) = match res {
                    Ok(id_modem) => id_modem,
                    Err(err) => {
                        error!("{err}");

                        return None;
                    }
                };

                if modem.apn.is_empty() && modem.imei.is_empty() && modem.imsi.is_empty() {
                    debug!("modem {id} fields are all empty");
                    None
                } else {
                    Some((id, modem))
                }
            })
            .collect()
            .await;

        Ok(properties)
    }

    pub async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        for (id, modem) in self.properties {
            set_property(client, INTERFACE, &format!("/{id}/apn"), modem.apn).await;
            set_property(client, INTERFACE, &format!("/{id}/imei"), modem.imei).await;
            set_property(client, INTERFACE, &format!("/{id}/imsi"), modem.imsi).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::types::AstarteData;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};

    #[tokio::test]
    async fn get_modem_properties_test() {
        let modem_id = "id";
        let modem = ModemProperties {
            apn: "apn".to_string(),
            imei: "imei".to_string(),
            imsi: "imsi".to_string(),
        };

        let mut client = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.CellularConnectionProperties"),
                predicate::eq("/id/apn"),
                predicate::eq(AstarteData::from("apn")),
            )
            .returning(|_, _, _| Ok(()));

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.CellularConnectionProperties"
                    && path == "/id/imei"
                    && *data == AstarteData::String("imei".to_string())
            })
            .returning(|_, _, _| Ok(()));

        client
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.CellularConnectionProperties"
                    && path == "/id/imsi"
                    && *data == AstarteData::String("imsi".to_string())
            })
            .returning(|_, _, _| Ok(()));

        CellularConnection {
            properties: HashMap::from([(modem_id.to_string(), modem)]),
        }
        .send(&mut client)
        .await;
    }
}
