// This file is part of Edgehog.
//
// Copyright 2022-2024 SECO Mind Srl
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

use std::{collections::HashMap, io};

use crate::data::{publish, Publisher};
use futures::TryFutureExt;
use log::{debug, error};
use serde::Deserialize;

async fn try_read_file(path: &str) -> io::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            error!("couldn't read {path}: {err}");

            Err(err)
        }
    }
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    line.split_once('=')
        .map(|(k, v)| (k, v.trim_matches('"')))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
}

pub struct OsRelease {
    pub os_info: OsInfo,
    pub base_image: BaseImage,
}

impl OsRelease {
    pub async fn read() -> Option<Self> {
        let content = try_read_file("/etc/os-release")
            .and_then(|file| async move {
                if file.is_some() {
                    Ok(file)
                } else {
                    // Only read this if the one in /etc doesn't exist
                    try_read_file("/usr/lib/os-release").await
                }
            })
            .await
            .ok()
            .flatten()?;

        Some(Self::from(content.as_str()))
    }

    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        self.os_info.send(client).await;
        self.base_image.send(client).await;
    }
}

impl From<&str> for OsRelease {
    fn from(s: &str) -> Self {
        let lines: HashMap<&str, &str> = s.lines().filter_map(split_key_value).collect();

        Self {
            os_info: OsInfo::from(&lines),
            base_image: BaseImage::from(&lines),
        }
    }
}

/// get structured data for `io.edgehog.devicemanager.OSInfo` interface
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub os_name: Option<String>,
    pub os_version: Option<String>,
}

impl OsInfo {
    const INTERFACE: &str = "io.edgehog.devicemanager.OSInfo";

    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        match self.os_name {
            Some(name) => {
                publish(client, Self::INTERFACE, "/osName", name).await;
            }
            None => {
                debug!("missing NAME in os-info");
            }
        }

        match self.os_version {
            Some(version) => {
                publish(client, Self::INTERFACE, "/osVersion", version).await;
            }
            None => {
                debug!("missing VERSION_ID or BUILD_ID in os-info");
            }
        }
    }
}

impl From<&HashMap<&str, &str>> for OsInfo {
    fn from(value: &HashMap<&str, &str>) -> Self {
        let name = value.get("NAME").map(|name| name.to_string());

        let version = value
            .get("VERSION_ID")
            .or_else(|| value.get("BUILD_ID"))
            .map(|version| version.to_string());

        Self {
            os_name: name,
            os_version: version,
        }
    }
}

#[derive(Debug, Default)]
pub struct BaseImage {
    name: Option<String>,
    version: Option<String>,
    build_id: Option<String>,
}

impl BaseImage {
    const INTERFACE: &str = "io.edgehog.devicemanager.BaseImage";

    pub async fn send<T>(self, client: &T)
    where
        T: Publisher,
    {
        match self.name {
            Some(name) => {
                publish(client, Self::INTERFACE, "/name", name).await;
            }
            None => {
                debug!("missing IMAGE_ID in os-info");
            }
        }

        match self.version {
            Some(version) => {
                publish(client, Self::INTERFACE, "/version", version).await;
            }
            None => {
                debug!("missing IMAGE_VERSION in os-info");
            }
        }

        match self.build_id {
            Some(build_id) => {
                publish(client, Self::INTERFACE, "/buildId", build_id).await;
            }
            None => {
                debug!("no build id set in IMAGE_VERSION");
            }
        }
    }
}

impl From<&HashMap<&str, &str>> for BaseImage {
    fn from(value: &HashMap<&str, &str>) -> Self {
        let name = value.get("IMAGE_ID").map(|name| name.to_string());

        let (version, build_id) = value
            .get("IMAGE_VERSION")
            // cursed some mapping
            .map(|version| match version.split_once('+') {
                Some((version, build_id)) => {
                    (Some(version.to_string()), Some(build_id.to_string()))
                }
                None => (Some(version.to_string()), None),
            })
            .unwrap_or_default();

        Self {
            name,
            version,
            build_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteType;

    use crate::data::tests::MockPubSub;

    use super::*;

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

        let data = OsRelease::from(file).os_info;
        assert_eq!(data.os_name.unwrap(), "Arch Linux");
        assert_eq!(data.os_version.unwrap(), "rolling");

        let file = r#"PRETTY_NAME="Debian GNU/Linux 11 (bullseye)"
NAME="Debian GNU/Linux"
VERSION_ID="11"
VERSION="11 (bullseye)"
VERSION_CODENAME=bullseye
ID=debian
HOME_URL="https://www.debian.org/"
SUPPORT_URL="https://www.debian.org/support"
BUG_REPORT_URL="https://bugs.debian.org/""#;

        let data = OsRelease::from(file).os_info;
        assert_eq!(data.os_name.unwrap(), "Debian GNU/Linux");
        assert_eq!(data.os_version.unwrap(), "11");
    }

    #[test]
    fn os_release_parsing_with_middle_empty_line() {
        let file = r#"NAME="Debian GNU/Linux"

VERSION_ID="11""#;

        let data = OsRelease::from(file).os_info;
        assert_eq!(data.os_name.unwrap(), "Debian GNU/Linux");
        assert_eq!(data.os_version.unwrap(), "11");
    }

    #[test]
    fn os_release_with_only_name() {
        let file = r#"NAME="Arch Linux"#;

        let data = OsRelease::from(file).os_info;
        assert_eq!(data.os_name.unwrap(), "Arch Linux");
        assert!(data.os_version.is_none());
    }

    #[test]
    fn os_release_malformed() {
        let file = r#"NAME["Arch Linux"@@"#;

        let data = OsRelease::from(file).os_info;
        assert!(data.os_name.is_none());
        assert!(data.os_version.is_none());
    }

    #[test]
    fn parse_key_value_line_empty() {
        let line = "";

        let data = split_key_value(line);
        assert!(data.is_none());
    }

    #[test]
    fn parse_key_value_line_malformed() {
        let line = r#"OS;"Arch"#;

        let data = split_key_value(line);
        assert!(data.is_none());
    }

    #[test]
    fn parse_key_value_line_valid() {
        let line = r#"OS="Arch"#;

        let (key, value) = split_key_value(line).unwrap();
        assert_eq!(key, "OS");
        assert_eq!(value, "Arch");
    }

    #[tokio::test]
    async fn should_send_os_info() {
        let mut mock = MockPubSub::new();

        mock.expect_send()
            .times(1)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.OSInfo"
                    && path == "/osName"
                    && *data == AstarteType::String("name".to_string())
            })
            .returning(|_, _, _| Ok(()));

        mock.expect_send()
            .times(1)
            .withf(|interface, path, data| {
                interface == "io.edgehog.devicemanager.OSInfo"
                    && path == "/osVersion"
                    && *data == AstarteType::String("version".to_string())
            })
            .returning(|_, _, _| Ok(()));

        OsInfo {
            os_name: Some("name".to_string()),
            os_version: Some("version".to_string()),
        }
        .send(&mock)
        .await;
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn get_base_image_test() {
        let result = OsRelease::read().await;
        assert!(result.is_some());
    }

    #[test]
    fn get_from_iter_empty_test() {
        const OS_RELEASE: &str = r#"
NAME="Ubuntu"
VERSION="18.04.6 LTS (Bionic Beaver)"
ID=ubuntu
ID_LIKE=debian
PRETTY_NAME="Ubuntu 18.04.6 LTS"
VERSION_ID="18.04"
HOME_URL="https://www.ubuntu.com/"
SUPPORT_URL="https://help.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
PRIVACY_POLICY_URL="https://www.ubuntu.com/legal/terms-and-policies/privacy-policy"
VERSION_CODENAME=bionic
UBUNTU_CODENAME=bionic"#;

        let base_image = OsRelease::from(OS_RELEASE).base_image;
        assert!(base_image.name.is_none());
        assert!(base_image.version.is_none());
        assert!(base_image.build_id.is_none());
    }

    #[test]
    fn get_from_iter_test() {
        const OS_RELEASE: &str = r#"
NAME="Ubuntu"
VERSION="18.04.6 LTS (Bionic Beaver)"
ID=ubuntu
ID_LIKE=debian
PRETTY_NAME="Ubuntu 18.04.6 LTS"
VERSION_ID="18.04"
HOME_URL="https://www.ubuntu.com/"
SUPPORT_URL="https://help.ubuntu.com/"
BUG_REPORT_URL="https://bugs.launchpad.net/ubuntu/"
PRIVACY_POLICY_URL="https://www.ubuntu.com/legal/terms-and-policies/privacy-policy"
VERSION_CODENAME=bionic
UBUNTU_CODENAME=bionic
IMAGE_ID="testOs"
IMAGE_VERSION="1.0.0+20220922""#;

        let base_image = OsRelease::from(OS_RELEASE).base_image;
        assert_eq!(base_image.name.unwrap(), "testOs");
        assert_eq!(base_image.version.unwrap(), "1.0.0");
        assert_eq!(base_image.build_id.unwrap(), "20220922");
    }
}
