// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
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

//! Container deployment models.

use std::fmt::Display;

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::{exists, BareSelect, Eq, Filter},
    expression::AsExpression,
    prelude::*,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
};

use super::container::Container;

use crate::{
    conversions::SqlUuid,
    schema::containers::{deployment_missing_containers, deployments},
};

/// Container deployment
#[derive(Debug, Clone, Copy, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::deployments)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Deployment {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Status of the deployment.
    pub status: DeploymentStatus,
}

type DeploymentById<'a> = Eq<deployments::id, &'a SqlUuid>;
type DeploymentFilterById<'a> = Filter<deployments::table, DeploymentById<'a>>;
type DeploymentExists<'a> = BareSelect<exists<Filter<deployments::table, DeploymentById<'a>>>>;

impl Deployment {
    /// Returns the filter deployment table by id.
    pub fn by_id(id: &SqlUuid) -> DeploymentById<'_> {
        deployments::id.eq(id)
    }

    /// Returns the filtered deployment table by id.
    pub fn find_id(id: &SqlUuid) -> DeploymentFilterById<'_> {
        deployments::table.filter(Self::by_id(id))
    }

    /// Returns the deployment exists query.
    pub fn exists(id: &SqlUuid) -> DeploymentExists<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Status of a deployment.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum DeploymentStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The deployment was acknowledged
    Published = 1,
    /// Up and running.
    Started = 2,
    /// Was stopped.
    Stopped = 3,
}

impl Display for DeploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeploymentStatus::Received => write!(f, "Received"),
            DeploymentStatus::Published => write!(f, "Published"),
            DeploymentStatus::Started => write!(f, "Started"),
            DeploymentStatus::Stopped => write!(f, "Stopped"),
        }
    }
}

impl From<DeploymentStatus> for i32 {
    fn from(value: DeploymentStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for DeploymentStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(DeploymentStatus::Received),
            1 => Ok(DeploymentStatus::Published),
            2 => Ok(DeploymentStatus::Started),
            3 => Ok(DeploymentStatus::Stopped),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for DeploymentStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for DeploymentStatus {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let val = i32::from(*self);

        out.set_value(val);

        Ok(IsNull::No)
    }
}

/// Container deployment
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::deployment_containers)]
#[diesel(belongs_to(Deployment))]
#[diesel(belongs_to(Container))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct DeploymentContainer {
    /// [`Deployment`] id
    pub deployment_id: SqlUuid,
    /// [`Container`] id
    pub container_id: SqlUuid,
}

/// Missing image for a container
#[derive(Debug, Clone, Copy, Insertable, Queryable, Associations, Selectable)]
#[diesel(table_name = crate::schema::containers::deployment_missing_containers)]
#[diesel(belongs_to(Deployment))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct DeploymentMissingContainer {
    /// [`Deployment`] id
    pub deployment_id: SqlUuid,
    /// [`Container`] id
    pub container_id: SqlUuid,
}

type DeploymentMissingContainerByContainer<'a> =
    Eq<deployment_missing_containers::container_id, &'a SqlUuid>;
type DeploymentMissingContainerFilterByContainer<'a> =
    Filter<deployment_missing_containers::table, DeploymentMissingContainerByContainer<'a>>;

impl DeploymentMissingContainer {
    /// Returns the filter deployment_missing_container table by id.
    pub fn by_container(container_id: &SqlUuid) -> DeploymentMissingContainerByContainer<'_> {
        deployment_missing_containers::container_id.eq(container_id)
    }

    /// Returns the filtered deployment_missing_container table by id.
    pub fn find_by_container(
        container_id: &SqlUuid,
    ) -> DeploymentMissingContainerFilterByContainer<'_> {
        deployment_missing_containers::table.filter(Self::by_container(container_id))
    }
}

#[cfg(test)]
mod tests {
    use super::DeploymentStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            DeploymentStatus::Received,
            DeploymentStatus::Published,
            DeploymentStatus::Started,
            DeploymentStatus::Stopped,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = DeploymentStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
