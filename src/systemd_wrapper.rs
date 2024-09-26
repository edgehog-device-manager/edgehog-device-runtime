// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Wrapper to notify systemd for the status

use std::io;

use systemd::daemon;
use systemd::daemon::{STATE_ERRNO, STATE_READY, STATE_STATUS};
use tracing::error;

/// Check the result of the call to [`daemon::notify`].
///
/// The notify function, internally calls `sd_notify` which can return:
/// - `result > 0` systemd was notified successfully
/// - `result < 0` with the corresponding io error
/// - `result = 0` couldn't contact systemd, it's probably not running
///
/// See more here [systemd sd-daemon.h](https://github.com/systemd/systemd/blob/000680a68dbdb07d77807868df0b4f978180e4cd/src/systemd/sd-daemon.h#L237)
fn check_notify_result(notify: Result<bool, io::Error>) {
    match notify {
        Ok(true) => {}
        Ok(false) => {
            // Result of `sd_notify == 0`, systemd is probably not running
            error!("couldn't notify systemd");
        }
        Err(err) => {
            error!("couldn't notify status to systemd: {err}");
        }
    }
}

pub fn systemd_notify_status(service_status: &str) {
    let systemd_state_pairs = [(STATE_STATUS, service_status)];
    let notify = daemon::notify(false, systemd_state_pairs.iter());

    check_notify_result(notify);
}

pub fn systemd_notify_ready_status(service_status: &str) {
    let systemd_state_pairs = [(STATE_READY, "1"), (STATE_STATUS, service_status)];
    let notify = daemon::notify(false, systemd_state_pairs.iter());

    check_notify_result(notify);
}

pub fn systemd_notify_errno_status(err_no: i32, service_status: &str) {
    let systemd_state_pairs = [
        (STATE_ERRNO, err_no.to_string()),
        (STATE_STATUS, service_status.to_string()),
    ];
    let notify = daemon::notify(false, systemd_state_pairs.iter());

    check_notify_result(notify);
}
