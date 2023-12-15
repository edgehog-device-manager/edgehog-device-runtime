// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::env;

use astarte_device_sdk::{
    builder::{DeviceBuilder, DeviceSdkBuild},
    store::memory::MemoryStore,
    transport::mqtt::{Credential, MqttConfig},
    types::AstarteType,
    DeviceEvent, Value,
};
use color_eyre::eyre::{self, Context};
use edgehog_docker::{service::Service, Docker};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use uuid::Uuid;

const DOCKER_REGISTRY: &str = "docker.io";

fn create_image(data: impl IntoIterator<Item = (impl Into<String>, AstarteType)>) -> DeviceEvent {
    let data = data.into_iter().map(|(k, v)| (k.into(), v)).collect();

    create_event(
        "io.edgehog.devicemanager.apps.CreateImageRequest",
        "/image",
        Value::Object(data),
    )
}

fn create_network(data: impl IntoIterator<Item = (impl Into<String>, AstarteType)>) -> DeviceEvent {
    let data = data.into_iter().map(|(k, v)| (k.into(), v)).collect();

    create_event(
        "io.edgehog.devicemanager.apps.CreateNetworkRequest",
        "/network",
        Value::Object(data),
    )
}

fn create_container(
    data: impl IntoIterator<Item = (impl Into<String>, AstarteType)>,
) -> DeviceEvent {
    let data = data.into_iter().map(|(k, v)| (k.into(), v)).collect();

    create_event(
        "io.edgehog.devicemanager.apps.CreateContainerRequest",
        "/container",
        Value::Object(data),
    )
}

fn create_event(interface: impl Into<String>, path: impl Into<String>, data: Value) -> DeviceEvent {
    DeviceEvent {
        interface: interface.into(),
        path: path.into(),
        data,
    }
}

fn id() -> String {
    Uuid::new_v4().to_string()
}

macro_rules! aty {
    ($v:expr) => {{
        let t: AstarteType = $v.into();

        t
    }};
    ($(($k:expr, $v:expr)),+$(,)?) => {
        [
            $(($k, aty!($v))),+
        ]
    };
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_default()
        .add_directive("edgehog_device_runtime_docker=DEBUG".parse()?);

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .try_init()?;

    let realm = env::var("ASTARTE_REALM_NAME").wrap_err("coulnd't read ASTARTE_REALM_NAME")?;
    let device_id = env::var("ASTARTE_DEVICE_ID").wrap_err("couldn't read ASTARTE_DEVICE_ID")?;
    let credentials_secret = env::var("ASTARTE_CREDENTIALS_SECRET")
        .wrap_err("couldn't read ASTARTE_CREDENTIALS_SECRET")?;
    let pairing_url =
        env::var("ASTARTE_PAIRING_URL").wrap_err("coulnd't read ASTARTE_PAIRING_URL")?;
    let interface_dir =
        env::var("ASTARTE_INTERFACE_DIR").wrap_err("coulnd't read ASTARTE_INTERFACE_DIR")?;

    let mut mqtt_config = MqttConfig::new(
        &realm,
        &device_id,
        Credential::secret(credentials_secret),
        &pairing_url,
    );
    mqtt_config.ignore_ssl_errors();

    // 3. Create the device instance
    let (device, _rx_events) = DeviceBuilder::new()
        .interface_directory(&interface_dir)?
        .store(MemoryStore::new())
        .connect(mqtt_config)
        .await?
        .build();

    let client = Docker::connect()?;

    let mut service = Service::new(client, device);

    let entry_bind = format!(
        "{}/scripts/entrypoint.sh:/entrypoint.sh",
        env!("CARGO_MANIFEST_DIR")
    );

    let nginx_img_id = id();
    let curl_img_id = id();
    let nginx_id = id();
    let curl_id = id();
    let net_id = id();

    let events: &[DeviceEvent] = &[
        create_image(aty![
            ("id", &nginx_img_id),
            ("repo", DOCKER_REGISTRY),
            ("name", "nginx"),
            ("tag", "stable-alpine-slim"),
        ]),
        create_image(aty![
            ("id", &curl_img_id),
            ("repo", DOCKER_REGISTRY),
            ("name", "curlimages/curl"),
            ("tag", "latest"),
        ]),
        create_network(aty![
            ("id", &net_id),
            ("driver", "bridge"),
            ("internal", false),
            ("checkDuplicate", true),
            ("enableIpv6", false),
        ]),
        create_container(aty![
            ("id", &nginx_id),
            ("imageId", &nginx_img_id),
            ("networkIds", vec![net_id.clone()]),
            ("networks", vec![net_id.clone()]),
            ("volumeIds", Vec::<String>::new()),
            (
                "image",
                format!("{DOCKER_REGISTRY}/nginx:stable-alpine-slim")
            ),
            ("hostname", "nginx"),
            ("restartPolicy", "no"),
            ("env", Vec::<String>::new()),
            ("binds", Vec::<String>::new()),
            ("portBindings", Vec::<String>::new()),
            ("privileged", false),
        ]),
        create_container(aty![
            ("id", &curl_id),
            ("hostname", "curl"),
            ("imageId", &curl_img_id),
            ("networkIds", vec![net_id.clone()]),
            ("networks", vec![net_id.clone()]),
            ("volumeIds", Vec::<String>::new()),
            ("image", format!("{DOCKER_REGISTRY}/curlimages/curl:latest")),
            ("restartPolicy", "no"),
            ("env", vec!["NGINX_HOST=nginx".to_string()]),
            ("binds", vec![entry_bind]),
            ("portBindings", Vec::<String>::new()),
            ("privileged", false),
        ]),
    ];

    for event in events {
        service.on_event(event.clone()).await?;
    }

    service.start(&nginx_id).await?;
    service.start(&curl_id).await?;

    Ok(())
}
