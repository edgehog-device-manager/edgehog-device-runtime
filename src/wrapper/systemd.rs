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

#[cfg(feature = "systemd")]
use systemd::daemon;
#[cfg(feature = "systemd")]
use systemd::daemon::{STATE_ERRNO, STATE_READY, STATE_STATUS};

#[allow(unused)]
pub fn systemd_notify_status(service_status: &str) {
    #[cfg(feature = "systemd")]
    {
        let systemd_state_pairs = vec![(STATE_STATUS, service_status)];
        daemon::notify(false, systemd_state_pairs.iter());
    }
}

#[allow(unused)]
pub fn systemd_notify_ready_status(service_status: &str) {
    #[cfg(feature = "systemd")]
    {
        let systemd_state_pairs = vec![(STATE_READY, "1"), (STATE_STATUS, service_status)];
        daemon::notify(false, systemd_state_pairs.iter());
    }
}

#[allow(unused)]
pub fn systemd_notify_errno_status(err_no: i32, service_status: &str) {
    #[cfg(feature = "systemd")]
    {
        let systemd_state_pairs = vec![
            (STATE_ERRNO, err_no.to_string()),
            (STATE_STATUS, service_status),
        ];
        daemon::notify(false, systemd_state_pairs.iter());
    }
}
