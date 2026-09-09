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

//! Parses an env file
//!
//! This follow the specification of env file as documented in the docker cli.
//!
//! # File format
//!
//! key/value files use the following syntax:
//!
//!   - File must be valid UTF-8.
//!   - (!!MISSING) BOM headers are removed.
//!   - Leading whitespace is removed for each line.
//!   - Lines starting with "#" are ignored.
//!   - Empty lines are ignored.
//!   - Key/Value pairs are provided as "KEY[=<VALUE>]".
//!   - Maximum line-length is limited to [64kb].
//!
//! # Interpolation, substitution, and escaping
//!
//! Both keys and values are used as-is; no interpolation, substitution or
//! escaping is supported, and quotes are considered part of the key or value.
//! Whitespace in values (including leading and trailing) is preserved. Given
//! that the file format is line-delimited, neither key, nor value, can contain
//! newlines.
//!
//! # Key/Value pairs
//!
//! Key/Value pairs take the following format:
//!
//! ```txt
//! KEY[=<VALUE>]
//! ```
//!
//! KEY is required and may not contain whitespaces or NUL characters. Any
//! other character (except for the "=" delimiter) are accepted, but  it is
//! recommended to use a subset of the POSIX portable character set, as
//! outlined in [Environment Variables].
//!
//! VALUE is optional, but may be empty. If no value is provided (i.e., no
//! equal sign ("=") is present), the KEY is omitted in the result, but some
//! functions accept a lookup-function to provide a default value for the
//! given key.
//!
//! [Environment Variables]: https://pubs.opengroup.org/onlinepubs/7908799/xbd/envvar.html

use std::fmt::Display;
use std::io;
use std::ops::ControlFlow;

use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Take};
use tracing::error;
use uuid::Uuid;

use crate::resource::ResourceError;

/// Line limit to read
const LIMIT: u64 = 64 * 1024;

pub(crate) enum EnvError {
    Io(io::Error),
    Env(&'static str),
}

impl EnvError {
    pub(crate) fn into_resource_err(self, id: Uuid) -> ResourceError {
        match self {
            EnvError::Io(error) => {
                error!(%error,"couldn't read env file");

                ResourceError::Invalid {
                    id,
                    resource: "env file",
                    ctx: "couldn't read env file to validate",
                }
            }
            EnvError::Env(error) => ResourceError::Invalid {
                id,
                resource: "env file",
                ctx: error,
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct EnvReader {
    inner: Take<BufReader<File>>,
    buf: String,
}

impl EnvReader {
    pub(crate) fn new(file: File) -> Self {
        Self {
            inner: BufReader::new(file).take(LIMIT),
            buf: String::new(),
        }
    }

    pub(crate) async fn validate(&mut self) -> Result<(), EnvError> {
        while let ControlFlow::Continue(_) = self.next_env().await? {}

        Ok(())
    }

    pub(crate) async fn next_env(
        &mut self,
    ) -> Result<ControlFlow<(), Option<KeyVal<'_>>>, EnvError> {
        self.buf.clear();
        // Reset the limit
        self.inner.set_limit(LIMIT);

        let read = self
            .inner
            .read_line(&mut self.buf)
            .await
            .map_err(EnvError::Io)?;

        if read == 0 {
            return Ok(ControlFlow::Break(()));
        }

        parse_line(&self.buf)
            .map(ControlFlow::Continue)
            .map_err(EnvError::Env)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct KeyVal<'a> {
    /// Env key
    pub(crate) key: &'a str,
    /// Optional value
    pub(crate) value: Option<&'a str>,
}

impl<'a> Display for KeyVal<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}", self.key, self.value.unwrap_or_default())
    }
}

pub(crate) fn parse_line(line: &str) -> Result<Option<KeyVal<'_>>, &'static str> {
    let line = line.trim_start();

    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }

    let (key, value) = match line.split_once('=') {
        Some((key, "")) => (key, None),
        Some((key, value)) => (key, Some(value)),
        None => (line, None),
    };

    if key.contains(|c: char| c.is_whitespace()) {
        return Err("key cannot container whitespaces");
    }

    Ok(Some(KeyVal { key, value }))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("# comment=", None)]
    #[case("VAR=VAR_VALUE", Some(KeyVal {  key: "VAR", value: Some("VAR_VALUE") }))]
    #[case("EMPTY_VAR=", Some(KeyVal { key: "EMPTY_VAR", value: None }))]
    #[case("UNDEFINED_VAR", Some(KeyVal {  key: "UNDEFINED_VAR", value: None }))]
    #[case("foo=bar", Some(KeyVal {  key: "foo", value: Some("bar") }))]
    #[case("    baz=quux", Some(KeyVal {  key: "baz", value: Some("quux") }))]
    #[case("# comment", None)]
    #[case("", None)]
    #[case("_foobar=foobaz", Some(KeyVal { key: "_foobar", value: Some("foobaz") }))]
    #[case("with.dots=working", Some(KeyVal { key: "with.dots", value: Some("working") }))]
    #[case("and_underscore=working too", Some(KeyVal { key: "and_underscore", value: Some("working too") }))]
    fn should_parse_envs(#[case] line: &str, #[case] exp: Option<KeyVal>) {
        let res = parse_line(line).unwrap();

        assert_eq!(res, exp);
    }
}
