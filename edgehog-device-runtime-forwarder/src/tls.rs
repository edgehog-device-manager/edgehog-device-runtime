// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Module implementing TLS functionality.
//!
//! This module provides the necessary functionalities to establish a TLS layer on top of the
//! WebSocket communication between a device and Edgehog.

use rustls::{ClientConfig, RootCertStore};
use std::io::BufReader;
use std::sync::Arc;

use tokio_tungstenite::Connector;
use tracing::{debug, info, instrument, warn};

/// TLS error
#[derive(displaydoc::Display, thiserror::Error, Debug)]
pub enum Error {
    /// Error while loading digital certificate or private key.
    WrongItem(String),

    /// Error while establishing a TLS connection.
    RustTls(#[from] rustls::Error),

    /// Error while accepting a TLS connection.
    AcceptTls(#[source] std::io::Error),

    /// Error while reading from file.
    ReadFile(#[source] std::io::Error),

    /// Read certificate
    ReadCert(#[source] std::io::Error),

    /// Failed to load root certificates
    LoadRootCert(#[source] std::io::Error),

    /// Couldn't load root certificate.
    RootCert(#[source] rustls::Error),
}

/// Given the CA certificate, compute the device TLS configuration and return a Device connector.
#[instrument(skip_all)]
pub fn device_tls_config() -> Result<Connector, Error> {
    let mut root_certs = RootCertStore::empty();

    // add native root certificates
    for cert in rustls_native_certs::load_native_certs().map_err(Error::LoadRootCert)? {
        root_certs.add(cert)?;
    }
    debug!("native root certificates added");

    // add custom roots certificate if necessary
    if let Some(ca_cert_file) = option_env!("EDGEHOG_FORWARDER_CA_PATH") {
        // I'm using std::fs because rustls-pemfile requires a sync read call
        info!("{ca_cert_file}");
        let file = std::fs::File::open(ca_cert_file).map_err(Error::ReadFile)?;
        let mut reader = BufReader::new(file);

        let certs = rustls_pemfile::certs(&mut reader);
        for cert in certs {
            let cert = cert.map_err(Error::ReadCert)?;
            root_certs.add(cert)?;
            debug!("added cert to root certificates");
        }
    }

    let config = ClientConfig::builder()
        .with_root_certificates(root_certs)
        .with_no_client_auth();

    Ok(Connector::Rustls(Arc::new(config)))
}
