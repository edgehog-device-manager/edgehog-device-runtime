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
use crate::repository::StateRepository;
use serde::{de::DeserializeOwned, Serialize};

pub struct FileStateRepository {
    pub path: String,
}

impl<T> StateRepository<T> for FileStateRepository
where
    T: Serialize + DeserializeOwned,
{
    fn write(&self, value: &T) -> Result<(), DeviceManagerError> {
        let mut file = std::fs::File::create(&self.path)?;
        let data_json = serde_json::to_string(value)?;
        std::io::Write::write_all(&mut file, data_json.as_bytes())?;
        Ok(())
    }

    fn read(&self) -> Result<T, DeviceManagerError> {
        let value_str = std::fs::read_to_string(&self.path)?;
        let value = serde_json::from_str(&value_str)?;
        Ok(value)
    }

    fn exists(&self) -> bool {
        std::path::Path::new(&self.path).exists()
    }

    fn clear(&self) -> Result<(), DeviceManagerError> {
        std::fs::remove_file(&self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::file_state_repository::FileStateRepository;
    use crate::repository::StateRepository;

    #[test]
    fn file_state_test() {
        let repository: Box<dyn StateRepository<i32>> = Box::new(FileStateRepository {
            path: "test.json".to_string(),
        });
        let value: i32 = 0;
        repository.write(&value).unwrap();
        assert!(repository.exists());
        assert_eq!(repository.read().unwrap(), value);
        repository.clear().unwrap();
    }
}
