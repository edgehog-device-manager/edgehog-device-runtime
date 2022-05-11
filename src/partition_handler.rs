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

use zbus::dbus_proxy;

#[dbus_proxy(
    interface = "io.edgehog.PartitionHandler1",
    default_service = "io.edgehog.PartitionHandler",
    default_path = "/io/edgehog/PartitionHandler"
)]
trait PartitionHandler {
    fn get_consecutive_fail_boot(&self) -> zbus::Result<String>;
    fn get_boot_fail_counter_state(&self) -> zbus::Result<String>;
    fn switch_partition(&self) -> zbus::Result<bool>;
}
