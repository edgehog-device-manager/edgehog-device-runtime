// Copyright 2023-2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use edgehog_forwarder::test_utils::con_manager;
use tokio::task::JoinSet;
use tracing::info;

#[derive(Parser, Debug)]
struct Cli {
    /// Host address of the forwarder server
    #[arg(long, short = 'H')]
    host: String,
    /// Port of the forwarder server
    #[arg(short, long, default_value_t = 4000)]
    port: u16,
    /// Session token
    #[arg(short, long)]
    token: String,
    /// Secure the connection using WSS
    #[arg(short, long, default_value_t = false)]
    secure: bool,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let Cli {
        host,
        port,
        token,
        secure,
    } = Cli::parse();

    let url = format!(
        "{}://{host}:{port}/device/websocket?session={token}",
        if secure { "wss" } else { "ws" }
    );
    let mut js = JoinSet::new();

    js.spawn(con_manager(url, secure));

    while let Some(res) = js.join_next().await {
        info!("{res:?}");
    }
}
