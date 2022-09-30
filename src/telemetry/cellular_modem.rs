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

use crate::DeviceManagerError;
use astarte_sdk::types::AstarteType;
use std::collections::HashMap;
use zbus::dbus_proxy;
use zbus::zvariant::{DeserializeDict, SerializeDict, Type};

#[derive(Debug, Clone, DeserializeDict, SerializeDict, Type)]
#[zvariant(signature = "dict")]
pub struct ModemProperties {
    apn: String,
    imei: String,
    imsi: String,
}

#[dbus_proxy(
    interface = "io.edgehog.CellularModems1",
    default_service = "io.edgehog.CellularModems",
    default_path = "/io/edgehog/CellularModems"
)]
trait CellularModems {
    fn list(&self) -> zbus::Result<Vec<String>>;
    fn get(&self, id: String) -> zbus::Result<ModemProperties>;
}

pub async fn get_cellular_properties() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let connection = match zbus::Connection::session().await {
        Ok(conn) => conn,
        Err(_) => return Ok(HashMap::new()),
    };
    let proxy = match CellularModemsProxy::new(&connection).await {
        Ok(p) => p,
        Err(_) => return Ok(HashMap::new()),
    };
    let available_modems = match proxy.list().await {
        Ok(modems) => modems,
        Err(_) => return Ok(HashMap::new()),
    };

    let mut ret: HashMap<String, AstarteType> = HashMap::new();
    for modem_id in available_modems {
        let modem: ModemProperties = proxy.get(modem_id.clone()).await?;
        ret.extend(get_modem_properties(modem_id, modem));
    }
    Ok(ret)
}

fn get_modem_properties(modem_id: String, modem: ModemProperties) -> HashMap<String, AstarteType> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();
    ret.insert(format!("/{}/apn", modem_id), AstarteType::String(modem.apn));
    ret.insert(
        format!("/{}/imei", modem_id),
        AstarteType::String(modem.imei),
    );
    ret.insert(
        format!("/{}/imsi", modem_id),
        AstarteType::String(modem.imsi),
    );
    ret
}

#[cfg(test)]
mod tests {
    use crate::telemetry::cellular_modem::{
        get_cellular_properties, get_modem_properties, ModemProperties,
    };
    use astarte_sdk::types::AstarteType;

    #[test]
    fn get_modem_properties_test() {
        let modem_id = "id";
        let modem = ModemProperties {
            apn: "apn".to_string(),
            imei: "imei".to_string(),
            imsi: "imsi".to_string(),
        };
        let modem_properties = get_modem_properties(modem_id.to_string(), modem);
        assert_eq!(
            modem_properties.get("/id/apn").unwrap(),
            &AstarteType::String("apn".to_string())
        );
        assert_eq!(
            modem_properties.get("/id/imei").unwrap(),
            &AstarteType::String("imei".to_string())
        );
        assert_eq!(
            modem_properties.get("/id/imsi").unwrap(),
            &AstarteType::String("imsi".to_string())
        );
    }
}
