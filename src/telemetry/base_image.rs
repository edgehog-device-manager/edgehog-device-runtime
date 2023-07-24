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

use std::collections::HashMap;

use astarte_device_sdk::types::AstarteType;

use crate::DeviceManagerError;

pub async fn get_base_image() -> Result<HashMap<String, AstarteType>, DeviceManagerError> {
    let file = tokio::fs::read_to_string("/etc/os-release").await?;

    Ok(file.lines().fold(HashMap::new(), get_from_iter))
}

fn get_from_iter(
    mut ret: HashMap<String, AstarteType>,
    line: &str,
) -> HashMap<String, AstarteType> {
    if let Some((key, value)) = line.trim().split_once('=') {
        match key {
            "IMAGE_ID" => {
                let value = value.replace('"', "");
                ret.insert("/name".to_string(), AstarteType::String(value));
            }
            "IMAGE_VERSION" => {
                let value = value.replace('"', "");
                if let Some((version, build_id)) = value.split_once('+') {
                    ret.insert(
                        "/version".to_string(),
                        AstarteType::String(version.to_string()),
                    );
                    ret.insert(
                        "/buildId".to_string(),
                        AstarteType::String(build_id.to_string()),
                    );
                } else {
                    ret.insert("/version".to_string(), AstarteType::String(value));
                }
            }
            _ => {}
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use crate::telemetry::base_image::{get_base_image, get_from_iter};
    use astarte_device_sdk::types::AstarteType;
    use std::collections::HashMap;

    #[tokio::test]
    async fn get_base_image_test() {
        let result = get_base_image();
        assert!(result.await.is_ok());
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

        let map = OS_RELEASE.lines().fold(HashMap::new(), get_from_iter);
        assert!(map.is_empty());
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

        let map = OS_RELEASE.lines().fold(HashMap::new(), get_from_iter);
        assert!(!map.is_empty());
        assert_eq!(
            map.get("/name").unwrap(),
            &AstarteType::String("testOs".to_string())
        );
        assert_eq!(
            map.get("/version").unwrap(),
            &AstarteType::String("1.0.0".to_string())
        );
        assert_eq!(
            map.get("/buildId").unwrap(),
            &AstarteType::String("20220922".to_string())
        );
    }
}
