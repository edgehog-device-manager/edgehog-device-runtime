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

use astarte_device_sdk::chrono::{DateTime, Utc};
use astarte_device_sdk::{Client, IntoAstarteObject};
use eyre::Context;
use tracing::{debug, error, instrument, trace};

use crate::data::send_object_with_timestamp;
use crate::telemetry::sender::TelemetryTask;

mod geoclue;

pub(crate) const INTERFACE: &str = "io.edgehog.devicemanager.Geolocation";

/// Generic Geolocation sampled data.
///
/// Geolocation allows geolocation sensors to stream location data, such as GPS data. Values
/// availability depends on what sensors are present on devices and what measurement systems are in
/// use. The id represents a unique identifier for an individual sensor.
#[derive(Debug, Clone, PartialEq, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct Geolocation {
    /// Sampled latitude value.
    #[astarte_object(fallible)]
    latitude: f64,
    /// Sampled longitude value.
    #[astarte_object(fallible)]
    longitude: f64,
    /// Sampled altitude value.
    #[astarte_object(fallible)]
    altitude: f64,
    /// Sampled accuracy of the latitude and longitude properties.
    #[astarte_object(fallible)]
    accuracy: f64,
    /// Sampled accuracy of the altitude property.
    #[astarte_object(fallible)]
    altitude_accuracy: f64,
    /// Sampled value representing the direction towards which the device is facing.
    #[astarte_object(fallible)]
    heading: f64,
    /// Sampled value representing the velocity of the device.
    #[astarte_object(fallible)]
    speed: f64,
}

#[derive(Debug, Default)]
pub(crate) struct GeolocationTelemetry {
    connection: Option<zbus::Connection>,
}

impl GeolocationTelemetry {
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
    async fn get_location(conn: &zbus::Connection) -> eyre::Result<(Geolocation, DateTime<Utc>)> {
        let manager = geoclue::ManagerProxy::new(conn)
            .await
            .wrap_err("couldn't get Manger")?;

        let client_path = manager
            .get_client()
            .await
            .wrap_err("couldn't get Client path")?;

        debug!(%client_path);

        let client = geoclue::ClientProxy::builder(conn)
            .path(client_path)?
            .build()
            .await?;

        // This is required
        client.set_desktop_id("edgehog-device-runtime").await?;

        client.start().await?;

        trace!("started location updates");

        let location_path = client.location().await?;

        debug!(%location_path);

        // Create a proxy for the Location object
        let location = geoclue::LocationProxy::builder(conn)
            .path(location_path)
            .wrap_err("location not available yet")?
            .build()
            .await?;

        // Read the properties
        let latitude = location.latitude().await?;
        let longitude = location.longitude().await?;
        let altitude = location.altitude().await?;
        let accuracy = location.accuracy().await?;
        let heading = location.heading().await?;
        let speed = location.speed().await?;
        let (ts_secs, ts_nanos) = location.timestamp().await?;

        let ts_secs = i64::try_from(ts_secs)
            .inspect_err(|error| error!(%error,ts_secs,"couldn't convert timestamp seconds"))
            .ok();
        let ts_nanos = u32::try_from(ts_nanos)
            .inspect_err(|error| error!(%error,ts_secs,"couldn't convert timestamp seconds"))
            .ok();

        let timestamp = ts_secs
            .zip(ts_nanos)
            .and_then(|(ts_secs, ts_nanos)| DateTime::from_timestamp(ts_secs, ts_nanos))
            .unwrap_or_else(Utc::now);

        Ok((
            Geolocation {
                latitude,
                longitude,
                altitude,
                accuracy,
                // NOTE: there's no separate altitude accuracy
                altitude_accuracy: accuracy,
                heading,
                speed,
            },
            timestamp,
        ))
    }
}

impl TelemetryTask for GeolocationTelemetry {
    #[instrument(skip_all)]
    async fn send<C>(&mut self, client: &mut C)
    where
        C: Client + Send,
    {
        let Some(conn) = self.connect().await else {
            return;
        };

        let (location, timestamp) = match Self::get_location(conn).await {
            Ok(location) => location,
            Err(error) => {
                error!(
                    error = %format_args!("{error:#}"),
                    "couldn't get geolocation"
                );

                return;
            }
        };

        trace!(?location, %timestamp);

        send_object_with_timestamp(client, INTERFACE, "/GeoClue2", location, timestamp).await;
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
        let geolocation = Geolocation {
            latitude: 1.1,
            longitude: 2.2,
            altitude: 3.3,
            accuracy: 4.4,
            altitude_accuracy: 5.5,
            heading: 6.6,
            speed: 7.7,
        };

        let obj = AstarteObject::try_from(geolocation).unwrap();

        with_insta!({
            assert_debug_snapshot!(obj);
        });
    }
}
