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

//! Models for all the resources.

use diesel::{
    dsl::{Eq, Filter},
    query_dsl::methods::FilterDsl,
    sql_types::Binary,
    AppearsOnTable, Expression, ExpressionMethods, Table,
};

use crate::conversions::SqlUuid;

#[cfg(feature = "containers")]
pub mod containers;

type ById<'a, Id> = Eq<Id, &'a SqlUuid>;
type FilterById<'a, Table, Id> = Filter<Table, ById<'a, Id>>;
// Only used in containers for now
#[cfg(feature = "containers")]
type ExistsFilterById<'a, Table, Id> =
    diesel::dsl::BareSelect<diesel::dsl::exists<FilterById<'a, Table, Id>>>;

/// General implementation for utility functions on a model.
pub trait QueryModel {
    /// Table to generate the queries for the model
    type Table: Table<PrimaryKey = Self::Id> + Default;
    /// Primary id of the table
    type Id: AppearsOnTable<Self::Table> + Expression<SqlType = Binary> + Default;

    /// Query type returned for the exists method.
    type ExistsQuery<'a>;

    /// Returns the filter table by id.
    fn by_id(id: &SqlUuid) -> ById<'_, Self::Id> {
        Self::Id::default().eq(id)
    }

    /// Returns the filtered table by id.
    fn find_id<'a>(id: &'a SqlUuid) -> FilterById<'a, Self::Table, Self::Id>
    where
        Self::Table: FilterDsl<ById<'a, Self::Id>>,
    {
        Self::Table::default().filter(Self::by_id(id))
    }

    /// Returns the deployment exists query.
    ///
    // TODO: this could be made into a default implementation too if the trait bound are satisfied
    fn exists(id: &SqlUuid) -> Self::ExistsQuery<'_>;
}
