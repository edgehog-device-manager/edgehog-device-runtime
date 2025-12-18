// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

//! Configure tls for the runtime.

use std::sync::Arc;

use rustls::crypto::CryptoProvider;
use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;
use tracing::{error, info, instrument};

/// Returns a configured TLS client
#[instrument]
pub(crate) fn config() -> Result<ClientConfig, rustls::Error> {
    // This step is to ensure a CryptoProvider is configured also in the tests.
    let provider = CryptoProvider::get_default()
        .map(Arc::clone)
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));

    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .inspect_err(|error| error!(%error, "couldn't configure safe default protocol versions"))?
        .with_platform_verifier()
        .inspect_err(|error| error!(%error, "couldn't configure platform verifier"))?
        .with_no_client_auth();

    info!("tls client configured with platform verifier");

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_default() {
        config().unwrap();
    }
}
