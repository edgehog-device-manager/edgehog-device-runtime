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

//! Container image models.

use std::fmt::Display;

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::{exists, BareSelect, Eq, Filter},
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
    ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
};

use crate::{conversions::SqlUuid, schema::containers::images};

/// Container image with the authentication to pull it.
#[derive(Debug, Clone, Insertable, Queryable, Selectable, PartialEq, Eq)]
#[diesel(table_name = crate::schema::containers::images)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(treat_none_as_default_value = false)]
pub struct Image {
    /// Unique id received from Edgehog.
    pub id: SqlUuid,
    /// Image id returned by the container engine.
    pub local_id: Option<String>,
    /// Status of the image.
    pub status: ImageStatus,
    /// Image reference to be pulled.
    ///
    /// It's in the form of: `docker.io/library/postgres:15-alpine`
    pub reference: String,
    /// Base64 encoded JSON for the registry auth.
    pub registry_auth: Option<String>,
}

type ImageById<'a> = Eq<images::id, &'a SqlUuid>;
type ImageFilterById<'a> = Filter<images::table, ImageById<'a>>;
type ImageExists<'a> = BareSelect<exists<Filter<images::table, ImageById<'a>>>>;

impl Image {
    /// Returns the filter image table by id.
    pub fn by_id(id: &SqlUuid) -> ImageById<'_> {
        images::id.eq(id)
    }

    /// Returns the filtered image table by id.
    pub fn find_id(id: &SqlUuid) -> ImageFilterById<'_> {
        images::table.filter(Self::by_id(id))
    }

    /// Returns the image exists query.
    pub fn exists(id: &SqlUuid) -> ImageExists<'_> {
        select(exists(Self::find_id(id)))
    }
}

/// Status of an image.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum ImageStatus {
    /// Received from Edgehog.
    #[default]
    Received = 0,
    /// The image was acknowledged
    Published = 1,
    /// Created on the runtime.
    Pulled = 2,
}

impl Display for ImageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageStatus::Received => write!(f, "Received"),
            ImageStatus::Published => write!(f, "Published"),
            ImageStatus::Pulled => write!(f, "Pulled"),
        }
    }
}

impl From<ImageStatus> for i32 {
    fn from(value: ImageStatus) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for ImageStatus {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ImageStatus::Received),
            1 => Ok(ImageStatus::Published),
            2 => Ok(ImageStatus::Pulled),
            _ => Err(format!("unrecognized status value {value}")),
        }
    }
}

impl FromSql<Integer, Sqlite> for ImageStatus {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for ImageStatus {
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
    use super::ImageStatus;

    #[test]
    fn should_convert_status() {
        let variants = [
            ImageStatus::Received,
            ImageStatus::Published,
            ImageStatus::Pulled,
        ];

        for exp in variants {
            let val = i32::from(exp);

            let status = ImageStatus::try_from(val).unwrap();

            assert_eq!(status, exp);
        }
    }
}
