// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

use tokio::task::JoinSet;
use tracing::info;

#[tokio::main]
async fn main() {
    use edgehog_forwarder::test_utils::con_manager;

    let mut js = JoinSet::new();

    js.spawn(con_manager(
        "ws://localhost:4000/device/websocket?session=abcd".to_string(),
        false,
    ));

    while let Some(res) = js.join_next().await {
        info!("{res:?}");
    }
}
