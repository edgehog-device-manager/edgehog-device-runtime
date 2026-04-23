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

use std::path::Path;

use clap::Subcommand;
use edgehog_store::db::Handle;
use edgehog_store::diesel::{HasQuery, RunQueryDsl};
use edgehog_store::models::job::Job;

#[derive(Debug, Clone, Copy, Subcommand)]
pub(crate) enum Jobs {
    List,
}

impl Jobs {
    pub(crate) async fn run(self, dir: &Path) -> eyre::Result<()> {
        let handle = Handle::open(dir.join("state.db")).await?;

        let jobs: Vec<Job> = handle
            .for_read(|reader| {
                let jobs = Job::query().load(reader)?;

                Ok(jobs)
            })
            .await?;

        println!("ID\tJOB_TYPE\tSTATUS\tVERSION\tTAG\tSCHEDULED_AT\tDATA");
        for job in jobs {
            let Job {
                id,
                job_type,
                status,
                version,
                tag,
                schedule_at,
                data,
            } = job;

            println!(
                "{id}\t{job_type:?}\t{status:?}\t{version}\t{tag}\t{schedule_at:?}\tcbor_len({})",
                data.len()
            )
        }

        Ok(())
    }
}
