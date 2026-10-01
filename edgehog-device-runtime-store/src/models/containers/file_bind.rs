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

//! Container file binds models.

use std::borrow::Cow;
use std::fmt::Display;
use std::str::FromStr;

use diesel::associations::Associations;
use diesel::backend::Backend;
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::dsl::{Eq, Filter, exists};
use diesel::expression::AsExpression;
use diesel::prelude::Insertable;
use diesel::serialize::{IsNull, ToSql};
use diesel::sql_types::{Integer, SmallInt};
use diesel::sqlite::Sqlite;
use diesel::{ExpressionMethods, HasQuery, QueryDsl, select};

use crate::conversions::SqlUuid;
use crate::models::containers::container::Container;
use crate::models::{ExistsFilterById, QueryModel};
use crate::schema::containers::{container_missing_env_files, container_missing_file_binds};

/// Container File Binds.
///
/// A bind the device must add to the container, for files sent through the file transfer.
#[derive(Debug, Clone, Insertable, HasQuery, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::file_binds)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct FileBind<'a> {
    /// File bind Id
    pub id: SqlUuid,
    /// Status of the file bind
    pub status: FileBindStatus,
    /// Source file target id.
    pub target_id: Cow<'a, str>,
    /// Source file target type.
    pub target_type: TargetType,
    /// File bind mount point in the container
    pub mountpoint: Cow<'a, str>,
    /// Source file target type.
    pub options: Option<Cow<'a, str>>,
}

impl<'f> QueryModel for FileBind<'f> {
    type Table = crate::schema::containers::file_binds::table;

    type Id = crate::schema::containers::file_binds::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// File binded by a container
#[derive(Debug, Clone, Copy, Insertable, HasQuery, Associations, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_file_binds)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(belongs_to(FileBind<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerFileBind {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`FileBind`] id
    pub file_bind_id: SqlUuid,
}

/// Missing file bind for a container
#[derive(Debug, Clone, Copy, Insertable, HasQuery, Associations, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_missing_file_binds)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingFileBind {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`FileBind`] id
    pub file_bind_id: SqlUuid,
}

type ContainerMissingFileBindByFileBind<'a> =
    Eq<container_missing_file_binds::file_bind_id, &'a SqlUuid>;
type ContainerMissingFileBindFilterByFileBind<'a> =
    Filter<container_missing_file_binds::table, ContainerMissingFileBindByFileBind<'a>>;

impl ContainerMissingFileBind {
    /// Returns the filter container_missing_file_binds table by id.
    pub fn by_file_bind(file_bind_id: &SqlUuid) -> ContainerMissingFileBindByFileBind<'_> {
        container_missing_file_binds::file_bind_id.eq(file_bind_id)
    }

    /// Returns the filtered container_missing_file_binds table by id.
    pub fn find_by_file_bind(
        file_bind_id: &SqlUuid,
    ) -> ContainerMissingFileBindFilterByFileBind<'_> {
        container_missing_file_binds::table.filter(Self::by_file_bind(file_bind_id))
    }
}

/// Container File Binds.
///
/// A bind the device must add to the container, for files sent through the file transfer.
#[derive(Debug, Clone, Insertable, HasQuery, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::env_files)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct EnvFile<'a> {
    /// File bind Id
    pub id: SqlUuid,
    /// Status of the env file
    pub status: EnvFileStatus,
    /// Source file target id.
    pub target_id: Cow<'a, str>,
    /// Source file target type.
    pub target_type: TargetType,
}

impl<'f> QueryModel for EnvFile<'f> {
    type Table = crate::schema::containers::env_files::table;

    type Id = crate::schema::containers::env_files::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Env file used by a container
#[derive(Debug, Clone, Copy, Insertable, HasQuery, Associations, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_env_files)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(belongs_to(EnvFile<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerEnvFile {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`EnvFile`] id
    pub env_file_id: SqlUuid,
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, HasQuery, Associations, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::container_missing_env_files)]
#[diesel(belongs_to(Container<'_>))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ContainerMissingEnvFile {
    /// [`Container`] id
    pub container_id: SqlUuid,
    /// [`EnvFile`] id
    pub env_file_id: SqlUuid,
}

type ContainerMissingEnvFileByEnvFile<'a> =
    Eq<container_missing_env_files::env_file_id, &'a SqlUuid>;
type ContainerMissingEnvFileFilterByEnvFile<'a> =
    Filter<container_missing_env_files::table, ContainerMissingEnvFileByEnvFile<'a>>;

impl ContainerMissingEnvFile {
    /// Returns the filter container_missing_env_files table by id.
    pub fn by_env_file(env_file_id: &SqlUuid) -> ContainerMissingEnvFileByEnvFile<'_> {
        container_missing_env_files::env_file_id.eq(env_file_id)
    }

    /// Returns the filtered container_missing_env_files table by id.
    pub fn find_by_env_file(env_file_id: &SqlUuid) -> ContainerMissingEnvFileFilterByEnvFile<'_> {
        container_missing_env_files::table.filter(Self::by_env_file(env_file_id))
    }
}

/// Status of the env file.
pub type EnvFileStatus = FileBindStatus;

/// Status of the file bind.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = SmallInt)]
pub enum FileBindStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// Acknowledged the file bind.
    Published = 1,
}

impl Display for FileBindStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileBindStatus::Received => write!(f, "Received"),
            FileBindStatus::Published => write!(f, "Published"),
        }
    }
}

impl From<FileBindStatus> for i32 {
    fn from(value: FileBindStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for FileBindStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FileBindStatus::Received),
            1 => Ok(FileBindStatus::Published),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<SmallInt, Sqlite> for FileBindStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<SmallInt, Sqlite> for FileBindStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// Target type for a file bind.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = Integer)]
pub enum TargetType {
    /// The file transfer store target
    Storage = 0,
}

impl Display for TargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetType::Storage => write!(f, "storage"),
        }
    }
}

impl FromStr for TargetType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let this = match s {
            "storage" => TargetType::Storage,
            _ => {
                return Err(());
            }
        };

        Ok(this)
    }
}

impl From<TargetType> for i32 {
    fn from(value: TargetType) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for TargetType {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(TargetType::Storage),
            _ => Err(format!("unrecognized target type value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for TargetType {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for TargetType {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_convert_target_type() {
        let variants = [TargetType::Storage];

        for exp in variants {
            let val = i32::from(exp);

            let res = TargetType::try_from(val).unwrap();

            assert_eq!(res, exp);
        }
    }

    #[test]
    fn should_parse_target_type() {
        let variants = [TargetType::Storage];

        for exp in variants {
            let val = exp.to_string();

            let res = TargetType::from_str(&val).unwrap();

            assert_eq!(res, exp);
        }
    }

    #[test]
    fn should_convert_status() {
        let variants = [FileBindStatus::Received, FileBindStatus::Published];

        for exp in variants {
            let val = i32::from(exp);

            let res = FileBindStatus::try_from(val).unwrap();

            assert_eq!(res, exp);
        }
    }
}
