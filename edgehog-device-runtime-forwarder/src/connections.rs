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

//! Collection of connections and respective methods.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use tokio::sync::mpsc::Sender;
use tracing::{debug, error, instrument};

use crate::connection::{Connection, ConnectionHandle};
use crate::connections_manager::Error;
use crate::messages::{HttpRequest, Id, ProtoMessage};

/// Collection of connections between the device and the bridge.
#[derive(Debug)]
pub(crate) struct Connections {
    /// Collection mapping every Connection ID with the corresponding [`tokio task`](tokio::task) spawned to
    /// handle it.
    connections: HashMap<Id, ConnectionHandle>,
    /// Write side of the channel used by each connection to send data to the [`ConnectionsManager`].
    /// This field is only cloned and passed to every connection when created.
    tx_ws: Sender<ProtoMessage>,
}

impl Connections {
    /// Initialize the Connections' collection.
    pub(crate) fn new(tx_ws: Sender<ProtoMessage>) -> Self {
        Self {
            connections: HashMap::new(),
            tx_ws,
        }
    }

    /// Create a new [`Connection`] in case a new HTTP request is received.
    #[instrument(skip(self, http_req))]
    pub(crate) fn handle_http(
        &mut self,
        request_id: Id,
        http_req: HttpRequest,
    ) -> Result<(), Error> {
        let tx_ws = self.tx_ws.clone();

        self.try_add(request_id.clone(), || {
            let request = http_req.request_builder()?;
            Ok(Connection::new(request_id, tx_ws, request).spawn())
        })
    }

    /// Return a connection entry only if is not finished.
    #[instrument(skip(self, f))]
    pub(crate) fn try_add<F>(&mut self, id: Id, f: F) -> Result<(), Error>
    where
        F: FnOnce() -> Result<ConnectionHandle, Error>,
    {
        // remove from the collection all the terminated connections
        self.remove_terminated();

        // check if there exist a connection with that id
        match self.connections.entry(id.clone()) {
            Entry::Occupied(mut entry) => {
                error!("entry already occupied");

                let handle = entry.get_mut();

                // check if the the connection is finished or not. If it is finished, return the
                // entry so that a new connection with the same Key can be created
                if !handle.is_finished() {
                    return Err(Error::IdAlreadyUsed(id));
                }

                debug!("connection terminated, replacing with a new connection");
                *handle = f()?;
            }
            Entry::Vacant(entry) => {
                debug!("vacant entry for");
                entry.insert(f()?);
            }
        }

        Ok(())
    }

    /// Remove all terminated connection from the connections' collection.
    #[instrument(skip_all)]
    pub(crate) fn remove_terminated(&mut self) {
        self.connections
            .retain(|_k, con_handle| !con_handle.is_finished());
    }

    /// Terminate all connections, both active and non.
    pub(crate) fn disconnect(&mut self) {
        self.connections.values_mut().for_each(|con| con.abort());
        self.connections.clear();
    }
}
