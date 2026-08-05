// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Cellular connection properties telemetry information.

use std::convert::identity;

use eyre::WrapErr;
use tracing::{debug, error, instrument};
use zbus::fdo::ObjectManagerProxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

use crate::Client;
use crate::data::set_property;
use crate::telemetry::stats::cellular::modem::{ModemProxy, SimProxy};

const INTERFACE: &str = "io.edgehog.devicemanager.CellularConnectionProperties";

#[instrument(skip_all)]
pub async fn send<C>(client: &mut C)
where
    C: Client,
{
    if let Err(error) = send_cellular_properties(client).await {
        error!(%error, "couldn't send cellular properties");
    }
}

#[derive(Debug, Clone, DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct ModemProperties {
    imei: String,
    imsi: Option<String>,
    apn: Option<String>,
}

impl ModemProperties {
    pub fn new(imei: String) -> Self {
        Self {
            imei,
            imsi: None,
            apn: None,
        }
    }

    pub async fn send<C>(self, client: &mut C, id: &str)
    where
        C: Client,
    {
        set_property(client, INTERFACE, &format!("/{id}/imei"), self.imei).await;

        if let Some(apn) = self.apn {
            set_property(client, INTERFACE, &format!("/{id}/apn"), apn).await;
        }

        if let Some(imsi) = self.imsi {
            set_property(client, INTERFACE, &format!("/{id}/imsi"), imsi).await;
        }
    }
}

async fn send_cellular_properties<C>(client: &mut C) -> eyre::Result<()>
where
    C: Client,
{
    let conn = zbus::Connection::system().await?;

    let object_manager = ObjectManagerProxy::new(
        &conn,
        "org.freedesktop.ModemManager1",
        "/org/freedesktop/ModemManager1",
    )
    .await?;

    let objects = object_manager.get_managed_objects().await?;

    for (path, interfaces) in objects {
        if !interfaces.contains_key("org.freedesktop.ModemManager1.Modem") {
            debug!("not a modem interface");

            continue;
        }

        debug!(%path, "reading modem");

        let modem_base = ModemProxy::builder(&conn)
            .path(&path)
            .wrap_err("invalid modem path")?
            .build()
            .await?;

        let device_id = modem_base
            .device_identifier()
            .await
            .wrap_err("couldn't get device identifier")?;

        let mut prop = modem_base
            .equipment_identifier()
            .await
            .map(ModemProperties::new)?;

        if let Err(error) = handle_sim(&mut prop, &conn, &modem_base).await {
            error!(%error, "couldn't get sim")
        }

        if let Err(error) = handle_bearers(&mut prop, &modem_base).await {
            error!(%error, "couldn't get apn")
        }

        debug!(%device_id, "sending modem");

        prop.send(client, &device_id).await;
    }

    Ok(())
}

#[instrument(skip_all)]
async fn handle_sim(
    prop: &mut ModemProperties,
    conn: &zbus::Connection,
    modem_base: &ModemProxy<'_>,
) -> Result<(), eyre::Error> {
    let sim_path = modem_base.sim().await?;

    if sim_path.as_ref() != "/" {
        debug!(%sim_path, "reading primary sim slot");

        let sim = SimProxy::builder(conn)
            .path(&sim_path)
            .wrap_err("invalid sim path")?
            .build()
            .await
            .wrap_err("couldn't build sim")?;

        prop.imsi = sim
            .imsi()
            .await
            .inspect_err(|error| error!(%error,"couldn't get SIM imsi"))
            .ok();
    }

    Ok(())
}

#[instrument(skip_all)]
async fn handle_bearers(
    prop: &mut ModemProperties,
    modem_base: &ModemProxy<'_>,
) -> eyre::Result<()> {
    for bearer in modem_base.bearers().await? {
        let mut bearer_prop = bearer
            .properties()
            .await
            .wrap_err("couldn't get bearer properties")?;

        let is_enabled = bearer_prop
            .remove("profile-enabled")
            .map(bool::try_from)
            .transpose()
            .wrap_err("couldn't convert profile-enabled")?
            .is_some_and(identity);

        if is_enabled {
            prop.apn = bearer_prop
                .remove("apn")
                .map(String::try_from)
                .transpose()
                .wrap_err("couldn't convert apn")?;

            if prop.apn.is_some() {
                return Ok(());
            }
        }
    }

    debug!("bearer apn not found");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::types::AstarteData;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::{Sequence, predicate};

    #[tokio::test]
    async fn get_modem_properties_test() {
        let modem_id = "id";
        let modem = ModemProperties {
            imei: "imei".to_string(),
            apn: Some("apn".to_string()),
            imsi: Some("imsi".to_string()),
        };

        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

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
                    && path == "/id/imsi"
                    && *data == AstarteData::String("imsi".to_string())
            })
            .returning(|_, _, _| Ok(()));

        modem.send(&mut client, modem_id).await;
    }
}
