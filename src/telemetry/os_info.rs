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

use crate::error::DeviceManagerError;
use astarte_sdk::types::AstarteType;
use std::collections::HashMap;

/// get structured data for `io.edgehog.devicemanager.OSInfo` interface
pub fn get_os_info() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let paths = ["/etc/os-release", "/usr/lib/os-release"];

    let paths = paths.iter().filter(|f| std::path::Path::new(f).exists());

    if let Some(path) = paths.into_iter().next() {
        let os = std::fs::read_to_string(path)?;
        return parse_os_info(&os);
    }

    Err(DeviceManagerError::FatalError(
        "No os-release file found".to_owned(),
    ))
}

fn parse_key_value_line(line: &str) -> Option<(&str, &str)> {
    let mut tokens = line.split('=');

    let key = tokens.next()?;
    let value = tokens.next()?.trim_matches('"');

    Some((key, value))
}

fn parse_os_info(os: &str) -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let mut ret: HashMap<String, AstarteType> = HashMap::new();

    let lines: HashMap<&str, &str> = os.lines().filter_map(parse_key_value_line).collect();

    if let Some(field) = lines.get("NAME") {
        ret.insert("/osName".to_owned(), field.into());
    }

    if let Some(field) = lines.get("VERSION_ID") {
        ret.insert("/osVersion".to_owned(), field.into());
    } else if let Some(field) = lines.get("BUILD_ID") {
        ret.insert("/osVersion".to_owned(), field.into());
    }
    Ok(ret)
}

#[cfg(test)]
mod tests {
    use crate::telemetry::os_info::{parse_key_value_line, parse_os_info};

    #[test]
    fn os_release_parsing() {
        let file = r#"NAME="Arch Linux"
PRETTY_NAME="Arch Linux"
ID=arch
BUILD_ID=rolling
ANSI_COLOR="38;2;23;147;209"
HOME_URL="https://archlinux.org/"
DOCUMENTATION_URL="https://wiki.archlinux.org/"
SUPPORT_URL="https://bbs.archlinux.org/"
BUG_REPORT_URL="https://bugs.archlinux.org/"
LOGO=archlinux-logo
"#;

        let data = parse_os_info(file).unwrap();
        assert_eq!(data["/osName"], "Arch Linux");
        assert_eq!(data["/osVersion"], "rolling");

        let file = r#"PRETTY_NAME="Debian GNU/Linux 11 (bullseye)"
NAME="Debian GNU/Linux"
VERSION_ID="11"
VERSION="11 (bullseye)"
VERSION_CODENAME=bullseye
ID=debian
HOME_URL="https://www.debian.org/"
SUPPORT_URL="https://www.debian.org/support"
BUG_REPORT_URL="https://bugs.debian.org/""#;

        let data = parse_os_info(file).unwrap();
        assert_eq!(data["/osName"], "Debian GNU/Linux");
        assert_eq!(data["/osVersion"], "11");
    }

    #[test]
    fn os_release_parsing_with_middle_empty_line() {
        let file = r#"NAME="Debian GNU/Linux"

VERSION_ID="11""#;

        let data = parse_os_info(file).unwrap();
        assert_eq!(data["/osName"], "Debian GNU/Linux");
        assert_eq!(data["/osVersion"], "11");
    }

    #[test]
    fn os_release_with_only_name() {
        let file = r#"NAME="Arch Linux"#;

        let data = parse_os_info(file).unwrap();
        assert_eq!(data["/osName"], "Arch Linux");
        assert!(!data.contains_key("/osVersion"));
    }

    #[test]
    fn os_release_malformed() {
        let file = r#"NAM["Arch Linux"@@"#;

        let data = parse_os_info(file).unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn parse_key_value_line_empty() {
        let line = "";

        let data = parse_key_value_line(line);
        assert!(data.is_none());
    }

    #[test]
    fn parse_key_value_line_malformed() {
        let line = r#"OS;"Arch"#;

        let data = parse_key_value_line(line);
        assert!(data.is_none());
    }

    #[test]
    fn parse_key_value_line_valid() {
        let line = r#"OS="Arch"#;

        let (key, value) = parse_key_value_line(line).unwrap();
        assert_eq!(key, "OS");
        assert_eq!(value, "Arch");
    }
}
