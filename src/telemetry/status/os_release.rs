// This file is part of Edgehog.
//
// Copyright 2022-2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, io};

use futures::TryFutureExt;
use serde::Deserialize;
use sysinfo::System;
use tracing::{debug, error};

use crate::Client;
use crate::data::set_property;

const OS_INFO_INTERFACE: &str = "io.edgehog.devicemanager.OSInfo";

const BASE_IMAGE_INTERFACE: &str = "io.edgehog.devicemanager.BaseImage";

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

fn parse_file(s: &str) -> HashMap<&str, &str> {
    s.lines().filter_map(split_key_value).collect()
}

pub struct OsRelease {
    pub os_info: OsInfo,
    pub base_image: BaseImage,
}

impl OsRelease {
    pub async fn read() -> Self {
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
            .flatten()
            .unwrap_or_default();

        let values = parse_file(&content);

        Self {
            os_info: OsInfo::read(&values),
            base_image: BaseImage::read(&values),
        }
    }

    pub async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        self.os_info.send(client).await;
        self.base_image.send(client).await;
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
    fn read(value: &HashMap<&str, &str>) -> Self {
        let name = System::name().or_else(|| value.get("NAME").map(|name| name.to_string()));

        let version = System::os_version().or_else(|| {
            value
                .get("VERSION_ID")
                .or_else(|| value.get("BUILD_ID"))
                .map(|version| version.to_string())
        });

        Self {
            os_name: name,
            os_version: version,
        }
    }

    pub async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        match self.os_name {
            Some(name) => {
                set_property(client, OS_INFO_INTERFACE, "/osName", name).await;
            }
            None => {
                debug!("missing NAME in os-info");
            }
        }

        match self.os_version {
            Some(version) => {
                set_property(client, OS_INFO_INTERFACE, "/osVersion", version).await;
            }
            None => {
                debug!("missing VERSION_ID or BUILD_ID in os-info");
            }
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
    fn read(value: &HashMap<&str, &str>) -> Self {
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

    pub async fn send<C>(self, client: &mut C)
    where
        C: Client,
    {
        match self.name {
            Some(name) => {
                set_property(client, BASE_IMAGE_INTERFACE, "/name", name).await;
            }
            None => {
                debug!("missing IMAGE_ID in os-info");
            }
        }

        match self.version {
            Some(version) => {
                set_property(client, BASE_IMAGE_INTERFACE, "/version", version).await;
            }
            None => {
                debug!("missing IMAGE_VERSION in os-info");
            }
        }

        match self.build_id {
            Some(build_id) => {
                set_property(client, BASE_IMAGE_INTERFACE, "/buildId", build_id).await;
            }
            None => {
                debug!("no build id set in IMAGE_VERSION");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::AstarteData;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use mockall::predicate;

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

        let data = parse_file(file);
        assert_eq!(*data.get("NAME").unwrap(), "Arch Linux");
        assert_eq!(*data.get("BUILD_ID").unwrap(), "rolling");

        let file = r#"PRETTY_NAME="Debian GNU/Linux 11 (bullseye)"
NAME="Debian GNU/Linux"
VERSION_ID="11"
VERSION="11 (bullseye)"
VERSION_CODENAME=bullseye
ID=debian
HOME_URL="https://www.debian.org/"
SUPPORT_URL="https://www.debian.org/support"
BUG_REPORT_URL="https://bugs.debian.org/""#;

        let data = parse_file(file);
        assert_eq!(*data.get("NAME").unwrap(), "Debian GNU/Linux");
        assert_eq!(*data.get("VERSION_ID").unwrap(), "11");
    }

    #[test]
    fn os_release_parsing_with_middle_empty_line() {
        let file = r#"NAME="Debian GNU/Linux"

VERSION_ID="11""#;

        let data = parse_file(file);
        assert_eq!(*data.get("NAME").unwrap(), "Debian GNU/Linux");
        assert_eq!(*data.get("VERSION_ID").unwrap(), "11");
    }

    #[test]
    fn os_release_with_only_name() {
        let file = r#"NAME="Arch Linux"#;

        let data = parse_file(file);
        assert!(data.contains_key("NAME"));
        assert!(!data.contains_key("VERSION_ID"));
    }

    #[test]
    fn os_release_malformed() {
        let file = r#"NAME["Arch Linux"@@"#;

        let data = parse_file(file);
        assert!(!data.contains_key("NAME"));
        assert!(!data.contains_key("VERSION_ID"));
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
        let mut mock = MockDeviceClient::<Mqtt<SqliteStore>>::new();

        mock.expect_set_property()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.OSInfo"),
                predicate::eq("/osName"),
                predicate::eq(AstarteData::String("name".to_string())),
            )
            .returning(|_, _, _| Ok(()));

        mock.expect_set_property()
            .once()
            .with(
                predicate::eq("io.edgehog.devicemanager.OSInfo"),
                predicate::eq("/osVersion"),
                predicate::eq(AstarteData::String("version".to_string())),
            )
            .returning(|_, _, _| Ok(()));

        OsInfo {
            os_name: Some("name".to_string()),
            os_version: Some("version".to_string()),
        }
        .send(&mut mock)
        .await;
    }

    #[tokio::test]
    async fn get_base_image_test() {
        OsRelease::read().await;
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

        let base_image = BaseImage::read(&parse_file(OS_RELEASE));
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

        let base_image = BaseImage::read(&parse_file(OS_RELEASE));
        assert_eq!(base_image.name.unwrap(), "testOs");
        assert_eq!(base_image.version.unwrap(), "1.0.0");
        assert_eq!(base_image.build_id.unwrap(), "20220922");
    }
}
