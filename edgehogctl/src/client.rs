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

use eyre::ensure;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ApiData<T> {
    pub(crate) data: T,
}

pub(crate) fn http_client() -> eyre::Result<reqwest::Client> {
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(astarte_device_tls::config()?)
        .build()?;

    Ok(client)
}

pub(crate) async fn gen_jwt_token() -> eyre::Result<String> {
    let token = tokio::process::Command::new("astartectl")
        .args(["utils", "gen-jwt", "all-realm-apis"])
        .output()
        .await?;

    ensure!(token.status.success(), "error in astartectl command");

    let token = std::str::from_utf8(&token.stdout)?.trim().to_string();

    Ok(token)
}
