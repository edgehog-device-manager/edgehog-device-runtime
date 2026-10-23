// This file is part of Edgehog.
//
// Copyright 2024-2026 SECO Mind Srl
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

//! Container image models.

use std::fmt::Display;
use std::str::FromStr;

use diesel::{
    Insertable, Queryable, Selectable,
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    dsl::exists,
    expression::AsExpression,
    select,
    serialize::{IsNull, ToSql},
    sql_types::Integer,
    sqlite::Sqlite,
};

use crate::{
    conversions::SqlUuid,
    models::{ExistsFilterById, QueryModel},
    schema::containers::images,
};

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
    /// Pulling policy for pulled images
    pub pull_policy: PullPolicy,
}

impl QueryModel for Image {
    type Table = images::table;

    type Id = images::id;

    type ExistsQuery<'a> = ExistsFilterById<'a, Self::Table, Self::Id>;

    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_> {
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

/// Pulling Policy of an image.
#[repr(u8)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression,
)]
#[diesel(sql_type = Integer)]
pub enum PullPolicy {
    /// Pull again only if not already pulled
    #[default]
    IfNotPresent = 0,
    /// Always pull
    Always = 1,
}

impl Display for PullPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PullPolicy::IfNotPresent => write!(f, "IfNotPresent"),
            PullPolicy::Always => write!(f, "Always"),
        }
    }
}

impl From<PullPolicy> for i32 {
    fn from(value: PullPolicy) -> Self {
        (value as u8).into()
    }
}

impl TryFrom<i32> for PullPolicy {
    type Error = String;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(PullPolicy::IfNotPresent),
            1 => Ok(PullPolicy::Always),

            _ => Err(format!("unrecognized pulling policy value {value}")),
        }
    }
}

impl FromStr for PullPolicy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Always" => Ok(Self::Always),
            "IfNotPresent" => Ok(Self::IfNotPresent),
            _ => Err(format!("the pull policy {s} is not supported")),
        }
    }
}

impl FromSql<Integer, Sqlite> for PullPolicy {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let value = i32::from_sql(bytes)?;

        Self::try_from(value).map_err(Into::into)
    }
}

impl ToSql<Integer, Sqlite> for PullPolicy {
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
    use std::str::FromStr;

    use crate::tests::with_insta;

    use super::{ImageStatus, PullPolicy};

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

            with_insta!({
                insta::assert_snapshot!(format!("{exp} = {val}"));
            });
        }
    }
    #[test]
    fn should_convert_pull_policy() {
        let variants = [PullPolicy::IfNotPresent, PullPolicy::Always];

        for exp in variants {
            let val = i32::from(exp);

            let pull_policy = PullPolicy::try_from(val).unwrap();

            assert_eq!(pull_policy, exp);

            with_insta!({
                insta::assert_snapshot!(format!("{exp} = {val}"));
            });
        }
    }

    #[test]
    fn should_error_unrecognized_pull_policy() {
        assert_eq!(
            PullPolicy::try_from(2).unwrap_err(),
            "unrecognized pulling policy value 2"
        );
    }

    #[rstest::rstest]
    #[case("IfNotPresent", Ok(PullPolicy::IfNotPresent))]
    #[case("Always", Ok(PullPolicy::Always))]
    #[case("NonSupportedPullPolicy", Err("the pull policy NonSupportedPullPolicy is not supported".to_string()))]
    fn from_str_to_pull_policy(#[case] pull_policy: &str, #[case] exp: Result<PullPolicy, String>) {
        assert_eq!(PullPolicy::from_str(pull_policy), exp);
    }
}
