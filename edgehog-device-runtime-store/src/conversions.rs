// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Conversions between rust and SQLITE/database types.

use std::{borrow::Borrow, fmt::Display, ops::Deref};

use diesel::{
    backend::Backend,
    deserialize::{FromSql, FromSqlRow},
    expression::AsExpression,
    serialize::{IsNull, ToSql},
    sql_types::{BigInt, Binary, SmallInt},
    sqlite::Sqlite,
};
use eyre::eyre;
use uuid::Uuid;

/// Binary serialization of a UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = Binary)]
pub struct SqlUuid(Uuid);

impl SqlUuid {
    /// create a new wrapped [`uuid`]
    pub fn new(uuid: impl Into<Uuid>) -> Self {
        Self(uuid.into())
    }
}

impl Deref for SqlUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<Uuid> for SqlUuid {
    fn borrow(&self) -> &Uuid {
        &self.0
    }
}

impl From<&Uuid> for SqlUuid {
    fn from(value: &Uuid) -> Self {
        SqlUuid(*value)
    }
}

impl From<Uuid> for SqlUuid {
    fn from(value: Uuid) -> Self {
        SqlUuid(value)
    }
}
impl From<SqlUuid> for Uuid {
    fn from(value: SqlUuid) -> Self {
        value.0
    }
}

impl Display for SqlUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromSql<Binary, Sqlite> for SqlUuid {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let data = Vec::<u8>::from_sql(bytes)?;

        Uuid::from_slice(&data).map(SqlUuid).map_err(Into::into)
    }
}

impl ToSql<Binary, Sqlite> for SqlUuid {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        let bytes = self.as_bytes().as_slice();

        <[u8] as ToSql<Binary, Sqlite>>::to_sql(bytes, out)
    }
}

/// Value of a container Swappiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = SmallInt)]
pub struct Swappiness(i16);

impl Swappiness {
    /// Checks for a valid Swappiness value.
    pub fn try_new<T>(value: T) -> Result<Option<Self>, i32>
    where
        T: Into<i32>,
    {
        let value = value.into();

        match value {
            ..=-1 => Ok(None),
            0..=100 => Ok(Some(Swappiness(value as i16))),
            value => Err(value),
        }
    }
}

impl Deref for Swappiness {
    type Target = i16;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromSql<SmallInt, Sqlite> for Swappiness {
    fn from_sql(value: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let data = i16::from_sql(value)?;

        let data = Self::try_new(data)
            .map_err(|err| eyre!("value should be between 0 and 100, but got: {err}"))?
            .ok_or_else(|| eyre!("value is out of range"))?;

        Ok(data)
    }
}

impl ToSql<SmallInt, Sqlite> for Swappiness {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        out.set_value(self.0 as i32);

        Ok(IsNull::No)
    }
}

/// Value of a container resource.
///
/// The UNSET is the minimum value for which the quota should be considered unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, FromSqlRow, AsExpression)]
#[diesel(sql_type = BigInt)]
pub struct QuotaValue<const UNSET: i64>(i64);

impl<const UNSET: i64> QuotaValue<UNSET> {
    /// Create a resource quota
    pub fn new(value: i64) -> Option<QuotaValue<UNSET>> {
        (value > UNSET).then_some(Self(value))
    }
}

impl<const UNSET: i64> Deref for QuotaValue<UNSET> {
    type Target = i64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<const UNSET: i64> FromSql<BigInt, Sqlite> for QuotaValue<UNSET> {
    fn from_sql(value: <Sqlite as Backend>::RawValue<'_>) -> diesel::deserialize::Result<Self> {
        let data = i64::from_sql(value)?;

        let data = Self::new(data)
            .ok_or_else(|| eyre!("quota unset is {UNSET}, value is out of range: {data}"))?;

        Ok(data)
    }
}

impl<const UNSET: i64> ToSql<BigInt, Sqlite> for QuotaValue<UNSET> {
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Sqlite>,
    ) -> diesel::serialize::Result {
        out.set_value(self.0);

        Ok(IsNull::No)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn should_convert_quota() {
        let cases = [
            (0, Some(QuotaValue::<-1>(0))),
            (1, Some(QuotaValue::<-1>(1))),
            (-1, None),
            (-2, None),
        ];

        for (case, exp) in cases {
            let res = QuotaValue::<-1>::new(case);

            assert_eq!(res, exp);
        }
    }

    #[test]
    fn should_convert_swappiness() {
        let cases = [
            (0, Ok(Some(Swappiness(0)))),
            (1, Ok(Some(Swappiness(1)))),
            (100, Ok(Some(Swappiness(100)))),
            (-1, Ok(None)),
            (-2, Ok(None)),
            (101, Err(101)),
        ];

        for (case, exp) in cases {
            let res = Swappiness::try_new(case);

            assert_eq!(res, exp);
        }
    }
}
