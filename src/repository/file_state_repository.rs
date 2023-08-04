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

use std::io;

use crate::error::DeviceManagerError;
use crate::repository::StateRepository;
use async_trait::async_trait;
use log::{debug, error};
use serde::{de::DeserializeOwned, Serialize};

pub struct FileStateRepository {
    pub path: String,
}

impl FileStateRepository {
    pub fn new(path: String, name: String) -> Self {
        let path = if path.ends_with('/') {
            path + &name
        } else {
            path + "/" + &name
        };
        FileStateRepository { path }
    }
}

#[async_trait]
impl<T> StateRepository<T> for FileStateRepository
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    async fn write(&self, value: &T) -> Result<(), DeviceManagerError> {
        let data_json = serde_json::to_string(value)?;
        tokio::fs::write(&self.path, &data_json).await?;
        Ok(())
    }

    async fn read(&self) -> Result<T, DeviceManagerError> {
        let value_str = tokio::fs::read_to_string(&self.path).await?;
        let value = serde_json::from_str(&value_str)?;
        Ok(value)
    }

    async fn exists(&self) -> bool {
        let metadata = match tokio::fs::metadata(&self.path).await {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                debug!("file doesn't exists");

                return false;
            }
            Err(err) => {
                error!("couldn't read state repository '{}': {}", self.path, err);

                return false;
            }
        };

        metadata.is_file()
    }

    async fn clear(&self) -> Result<(), DeviceManagerError> {
        tokio::fs::remove_file(&self.path).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::repository::file_state_repository::FileStateRepository;
    use crate::repository::StateRepository;

    #[tokio::test]
    async fn file_state_test() {
        let dir = tempdir::TempDir::new("edgehog").expect("failed to create temp dir");
        let mut repo = dir.into_path();
        repo.push("test.json");
        let path = repo.to_string_lossy().to_string();

        let repository: Box<dyn StateRepository<i32>> = Box::new(FileStateRepository { path });

        let value: i32 = 0;
        repository.write(&value).await.unwrap();
        assert!(repository.exists().await);
        assert_eq!(repository.read().await.unwrap(), value);
        repository.clear().await.unwrap();
    }

    #[test]
    fn file_repository_new_end_without_slash() {
        let file = FileStateRepository::new("/tmp/path".to_owned(), "state.json".to_owned());

        assert_eq!(file.path, "/tmp/path/state.json".to_owned())
    }

    #[test]
    fn file_repository_new_end_with_slash() {
        let file = FileStateRepository::new("/tmp/path/".to_owned(), "state.json".to_owned());

        assert_eq!(file.path, "/tmp/path/state.json".to_owned())
    }
}
