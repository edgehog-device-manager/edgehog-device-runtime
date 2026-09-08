// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Location telemetry.

use std::fmt::Display;

use astarte_device_sdk::chrono::Utc;
use astarte_device_sdk::{AstarteData, Client, IntoAstarteObject};
use eyre::Context;
use tracing::{debug, error, instrument};
use zbus::fdo::ObjectManagerProxy;

use crate::data::send_object_with_timestamp;
use crate::telemetry::sender::TelemetryTask;

use self::modem::{LocationProxy, ModemProxy, SimpleProxy};

pub(crate) mod modem;

pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.CellularConnectionStatus";

/// Generic Geolocation sampled data.
///
/// Geolocation allows geolocation sensors to stream location data, such as GPS data. Values
/// availability depends on what sensors are present on devices and what measurement systems are in
/// use. The id represents a unique identifier for an individual sensor.
#[derive(Debug, Clone, Default, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct CellularConnectionStatus {
    /// Connectivity carrier operator name.
    #[astarte_object(required = false)]
    carrier: Option<String>,
    /// The Cell ID in hexadecimal format.
    ///
    /// Either 16 bit for 2G or 28 bit for 3G or 4G.
    #[astarte_object(required = false)]
    cell_id: Option<i64>,
    /// The mobile country code (MCC) for the device's home network.
    ///
    /// Valid range: 0–999.
    #[astarte_object(required = false)]
    mobile_country_code: Option<i32>,
    /// The Mobile Network Code for the device's home network.
    ///
    /// This is the MNC for GSM, WCDMA, LTE and NR. CDMA uses the System ID (SID).
    ///
    /// Valid range for MNC: 0–999. Valid range for SID: 0–32767.
    #[astarte_object(required = false)]
    mobile_network_code: Option<i32>,
    /// Two byte location area code in hexadecimal format.
    #[astarte_object(required = false)]
    local_area_code: Option<i32>,
    /// GSM/LTE registration status.
    #[astarte_object(required = false)]
    registration_status: Option<RegistrationStatus>,
    /// Signal strength of the device in dBm.
    #[astarte_object(fallible, required = false)]
    rssi: Option<f64>,
    /// Access technology.
    #[astarte_object(required = false)]
    technology: Option<Technology>,
}

impl CellularConnectionStatus {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Registration State
#[derive(Debug, Clone, PartialEq)]
enum RegistrationStatus {
    /// Not registered, not searching for new operator to register.
    Idle,
    /// Registered on home network.
    Home,
    /// Not registered, searching for new operator to register with.
    Searching,
    /// Registration denied.
    Denied,
    /// Unknown registration status.
    Unknown,
    /// Registered on a roaming network.
    Roaming,
    /// Registered for "SMS only", home network (applicable only when on LTE).
    HomeSmsOnly,
    /// Registered for "SMS only", roaming network (applicable only when on LTE).
    RoamingSmsOnly,
    /// Emergency services only.
    EmergencyOnly,
    /// Registered for "CSFB not preferred", home network (applicable only when on LTE).
    HomeCsfbNotPreferred,
    /// Registered for "CSFB not preferred", roaming network (applicable only when on LTE).
    RoamingCsfbNotPreferred,
    /// Attached for access to Restricted Local Operator Services (applicable only when on LTE).
    AttachedRlos,
    // CDMA Registered state
    Registered,
}

impl RegistrationStatus {
    fn from_m3gpp(value: u32) -> Option<RegistrationStatus> {
        let value = match value {
            0 => RegistrationStatus::Idle,
            1 => RegistrationStatus::Home,
            2 => RegistrationStatus::Searching,
            3 => RegistrationStatus::Denied,
            4 => RegistrationStatus::Unknown,
            5 => RegistrationStatus::Roaming,
            6 => RegistrationStatus::HomeSmsOnly,
            7 => RegistrationStatus::RoamingSmsOnly,
            8 => RegistrationStatus::EmergencyOnly,
            9 => RegistrationStatus::HomeCsfbNotPreferred,
            10 => RegistrationStatus::RoamingCsfbNotPreferred,
            11 => RegistrationStatus::AttachedRlos,
            _ => {
                error!(value, "unhandled m3gpp registration status");

                return None;
            }
        };

        Some(value)
    }

    fn from_cdma(value: u32) -> Option<RegistrationStatus> {
        let value = match value {
            0 => RegistrationStatus::Unknown,
            1 => RegistrationStatus::Registered,
            2 => RegistrationStatus::Home,
            3 => RegistrationStatus::Roaming,
            _ => {
                error!(value, "unhandled cdma registration status");

                return None;
            }
        };

        Some(value)
    }
}

impl Display for RegistrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: update the astarte interface to support the additional values
        match self {
            RegistrationStatus::Idle
            | RegistrationStatus::EmergencyOnly
            | RegistrationStatus::AttachedRlos => {
                write!(f, "NotRegistered")
            }
            RegistrationStatus::Home
            | RegistrationStatus::HomeSmsOnly
            | RegistrationStatus::HomeCsfbNotPreferred
            | RegistrationStatus::Registered => write!(f, "Registered"),
            RegistrationStatus::Searching => write!(f, "SearchingOperator"),
            RegistrationStatus::Denied => write!(f, "RegistrationDenied"),
            RegistrationStatus::Unknown => write!(f, "Unknown"),
            RegistrationStatus::Roaming
            | RegistrationStatus::RoamingSmsOnly
            | RegistrationStatus::RoamingCsfbNotPreferred => {
                write!(f, "RegisteredRoaming")
            }
        }
    }
}

impl From<RegistrationStatus> for AstarteData {
    fn from(value: RegistrationStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

/// Modem Access Technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum Technology {
    /// The access technology used is unknown.
    Unknown = 0,
    /// Analog wireline telephone.
    Pots = 1 << 0,
    /// GSM.
    Gsm = 1 << 1,
    /// Compact GSM.
    GsmCompact = 1 << 2,
    /// GPRS.
    Gprs = 1 << 3,
    /// EDGE (ETSI 27.007: "GSM w/EGPRS").
    Edge = 1 << 4,
    /// UMTS (ETSI 27.007: "UTRAN").
    Umts = 1 << 5,
    /// HSDPA (ETSI 27.007: "UTRAN w/HSDPA").
    Hsdpa = 1 << 6,
    /// HSUPA (ETSI 27.007: "UTRAN w/HSUPA").
    Hsupa = 1 << 7,
    /// HSPA (ETSI 27.007: "UTRAN w/HSDPA and HSUPA").
    Hspa = 1 << 8,
    /// HSPA+ (ETSI 27.007: "UTRAN w/HSPA+").
    HspaPlus = 1 << 9,
    /// CDMA2000 1xRTT.
    _1xRtt = 1 << 10,
    /// CDMA2000 EVDO revision 0.
    Evdo0 = 1 << 11,
    /// CDMA2000 EVDO revision A.
    EvdoA = 1 << 12,
    /// CDMA2000 EVDO revision B.
    EvdoB = 1 << 13,
    /// LTE (ETSI 27.007: "E-UTRAN")
    Lte = 1 << 14,
    /// 5GNR (ETSI 27.007: "NG-RAN").
    _5GNr = 1 << 15,
    /// Cat-M (ETSI 23.401: LTE Category M1/M2).
    LteCatM = 1 << 16,
    /// NB IoT (ETSI 23.401: LTE Category NB1/NB2).
    LteNbIot = 1 << 17,
    /// Mask specifying all access technologies
    Any = 0xFFFFFFFF,
}

impl Technology {
    fn from_u32(value: u32) -> Option<Technology> {
        let tech = match value.trailing_zeros() {
            32 => Technology::Unknown,
            0 if value == 0xFFFFFFFF => Technology::Any,
            0 if value == 1 => Technology::Pots,
            1 => Technology::Gsm,
            2 => Technology::GsmCompact,
            3 => Technology::Gprs,
            4 => Technology::Edge,
            5 => Technology::Umts,
            6 => Technology::Hsdpa,
            7 => Technology::Hsupa,
            8 => Technology::Hspa,
            9 => Technology::HspaPlus,
            10 => Technology::_1xRtt,
            11 => Technology::Evdo0,
            12 => Technology::EvdoA,
            13 => Technology::EvdoB,
            14 => Technology::Lte,
            15 => Technology::_5GNr,
            16 => Technology::LteCatM,
            17 => Technology::LteNbIot,
            _ => {
                error!(value, "unhandled modem access technology");

                return None;
            }
        };

        Some(tech)
    }
}

impl Display for Technology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: update the astarte interface to support the additional values
        match self {
            Technology::Gsm | Technology::Gprs => write!(f, "GSM"),
            Technology::GsmCompact => write!(f, "GSMCompact"),
            Technology::Umts => write!(f, "UTRAN"),
            Technology::Edge => write!(f, "GSMwEGPRS"),
            Technology::Hsdpa => write!(f, "UTRANwHSDPA"),
            Technology::Hsupa => write!(f, "UTRANwHSUPA"),
            Technology::Hspa | Technology::HspaPlus => write!(f, "UTRANwHSDPAandHSUPA"),
            Technology::Lte | Technology::LteNbIot | Technology::LteCatM => write!(f, "EUTRAN"),
            // TODO: fallback since completely incompatible
            Technology::_1xRtt | Technology::Evdo0 | Technology::EvdoA | Technology::EvdoB => {
                write!(f, "GSM")
            }
            // TODO: this is just as a fall back
            Technology::Any | Technology::Unknown | Technology::Pots | Technology::_5GNr => {
                write!(f, "EUTRAN")
            }
        }
    }
}

impl From<Technology> for AstarteData {
    fn from(value: Technology) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[derive(Debug, Default)]
pub(crate) struct CellularConnectionStatusTelemetry {
    connection: Option<zbus::Connection>,
}

impl CellularConnectionStatusTelemetry {
    pub(crate) async fn connect(&mut self) -> Option<&zbus::Connection> {
        if self.connection.is_none() {
            let connection = match zbus::Connection::system().await {
                Ok(conn) => conn,
                Err(error) => {
                    error!(%error, "couldn't connect to system dbus");

                    return None;
                }
            };

            self.connection = Some(connection);
        }

        self.connection.as_ref()
    }

    #[instrument(skip_all)]
    async fn send_statuses<C>(client: &mut C, conn: &zbus::Connection) -> eyre::Result<()>
    where
        C: Client + Send,
    {
        let object_manager = ObjectManagerProxy::new(
            conn,
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

            let mut status = CellularConnectionStatus::default();

            let modem_base = ModemProxy::builder(conn)
                .path(&path)
                .wrap_err("invalid modem path")?
                .build()
                .await?;

            let device_id = modem_base.device_identifier().await?;

            if interfaces.contains_key("org.freedesktop.ModemManager1.Modem.Simple") {
                let modem_simple = SimpleProxy::builder(conn).path(&path)?.build().await?;

                Self::handle_modem_simple(&mut status, &modem_simple).await?;
            }

            if interfaces.contains_key("org.freedesktop.ModemManager1.Modem.Location") {
                let modem_location = LocationProxy::builder(conn).path(&path)?.build().await?;

                Self::handle_modem_location(&mut status, &modem_location).await?;
            }

            if !status.is_empty() {
                debug!("sending modem status");

                send_object_with_timestamp(
                    client,
                    INTERFACE,
                    &format!("/{device_id}"),
                    status,
                    Utc::now(),
                )
                .await;
            } else {
                debug!("modem status is empty, skipping");
            }
        }

        Ok(())
    }

    async fn handle_modem_simple(
        status: &mut CellularConnectionStatus,
        modem_simple: &SimpleProxy<'_>,
    ) -> eyre::Result<()> {
        let mut modem_status = modem_simple
            .get_status()
            .await
            .wrap_err("couldn't get modem simple status")?;

        status.rssi = modem_status
            .remove("signal-quality")
            .map(|value| -> eyre::Result<f64> {
                // signature `(ub)`
                let (value, recent): (u32, bool) = value
                    .try_into()
                    .wrap_err("couldn't convert signal-quality")?;

                if !recent {
                    debug!("not a recent signal quality measurement");
                }

                Ok(f64::from(value))
            })
            .transpose()?;

        status.technology = modem_status
            .remove("access-technologies")
            .map(|tech| -> eyre::Result<u32> {
                // signature `u`
                u32::try_from(tech).wrap_err("couldn't convert access-technologies")
            })
            .transpose()?
            .and_then(Technology::from_u32);

        let reg_status = modem_status
            .remove("m3gpp-registration-state")
            .map(|state| {
                // signature `u`
                u32::try_from(state).wrap_err("couldn't convert m3gpp-registration-state")
            })
            .transpose()?;

        status.registration_status = match reg_status {
            Some(value) => {
                debug!("using m3gpp-registration-state");

                RegistrationStatus::from_m3gpp(value)
            }
            None => {
                debug!("checking cdma-cdma1x-registration-state");

                modem_status
                    .remove("cdma-cdma1x-registration-state")
                    .or_else(|| {
                        debug!("checking cdma-evdo-registration-state");

                        modem_status.remove("cdma-evdo-registration-state")
                    })
                    .map(|state| {
                        // signature `u`
                        u32::try_from(state)
                            .wrap_err("couldn't convert cdma-cdma1x-registration-state")
                    })
                    .transpose()?
                    .and_then(RegistrationStatus::from_cdma)
            }
        };

        let operator_code = modem_status
            .remove("m3gpp-operator-code")
            .map(|operator_code| {
                String::try_from(operator_code).wrap_err("couldn't convert m3gpp-operator-code")
            })
            .transpose()?;

        if let Some(operator_code) = operator_code {
            debug!(operator_code, "got operator code");

            //  Returned in the format "MCCMNC", where MCC is the three-digit ITU E.212
            //  Mobile Country Code and MNC is the two- or three-digit GSM Mobile Network Code.
            //  e.g. e"31026" or "310260". If the MCC and MNC are not known or the mobile is not
            //  registered to a mobile network, this property will be a zero-length (blank)
            //  string.
            if let Some((mcc, mnc)) = operator_code.split_at_checked(3) {
                status.mobile_country_code = mcc
                    .parse()
                    .inspect_err(|error| error!(%error, "couldn't parse mobile country code"))
                    .ok();
                status.mobile_network_code = mnc
                    .parse()
                    .inspect_err(|error| error!(%error, "couldn't parse mobile network code"))
                    .ok();
            } else {
                error!(operator_code, "invalid operator code");
            }
        }

        status.carrier = modem_status
            .remove("m3gpp-operator-name")
            .map(|name| String::try_from(name).wrap_err("couldn't convert m3gpp-operator-name"))
            .transpose()?;

        Ok(())
    }

    async fn handle_modem_location(
        status: &mut CellularConnectionStatus,
        modem_location: &LocationProxy<'_>,
    ) -> eyre::Result<()> {
        // See <https://gitlab.freedesktop.org/mobile-broadband/ModemManager/-/blob/main/include/ModemManager-enums.h>
        const MM_MODEM_LOCATION_SOURCE_3GPP_LAC_CI: u32 = 1 << 0;

        let mut location = modem_location
            .get_location()
            .await
            .wrap_err("couldn't get location")?;

        let lac_ci = location
            .remove(&MM_MODEM_LOCATION_SOURCE_3GPP_LAC_CI)
            .map(|lac_ci| String::try_from(lac_ci).wrap_err("couldn't convert LAC"))
            .transpose()?;

        //  Devices supporting this capability return a string in the format
        //  "MCC,MNC,LAC,CI,TAC" (without the quotes of course) where the following applies:
        //
        // - LAC: This is the two-byte Location Area Code of the GSM/UMTS base station with
        //   which the mobile is registered, in upper-case hexadecimal format without
        //   leading zeros, as specified in 3GPP TS 27.007. E.g. "84CD".
        //
        // - CI: This is the two- or four-byte Cell Identifier with which the mobile is
        //   registered, in upper-case hexadecimal format without leading zeros, as specified
        //   in 3GPP TS 27.007. e.g. "2BAF" or "D30156".
        if let Some(lac_ci) = lac_ci {
            let mut iter = lac_ci.split(',').skip(2);

            status.local_area_code = iter.next().and_then(|lac| {
                i32::from_str_radix(lac, 16)
                    .inspect_err(|error| error!(%error, "couldn't parse LAC"))
                    .ok()
            });

            status.cell_id = iter.next().and_then(|lac| {
                i64::from_str_radix(lac, 16)
                    .inspect_err(|error| error!(%error, "couldn't parse CI"))
                    .ok()
            });
        }

        Ok(())
    }
}

impl TelemetryTask for CellularConnectionStatusTelemetry {
    #[instrument(skip_all)]
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send,
    {
        let Some(conn) = self.connect().await else {
            return;
        };

        if let Err(error) = Self::send_statuses(client, conn).await {
            error!(
                error = %format_args!("{error:#}"),
                "couldn't get geolocation"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use insta::assert_debug_snapshot;

    use crate::tests::with_insta;

    use super::*;

    #[test]
    fn should_convert_to_data() {
        let geolocation = CellularConnectionStatus {
            carrier: Some("Test".to_string()),
            // Should be 16 bit
            cell_id: Some(16),
            mobile_country_code: Some(42),
            mobile_network_code: Some(24),
            //  2 bytes
            local_area_code: Some(2),
            registration_status: Some(RegistrationStatus::Idle),
            rssi: Some(10.0),
            technology: Some(Technology::Gsm),
        };

        let obj = AstarteObject::try_from(geolocation).unwrap();

        with_insta!({
            assert_debug_snapshot!(obj);
        });
    }
}
