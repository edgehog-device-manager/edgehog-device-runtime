// Copyright 2023 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Collection of connections and respective methods.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};

use tokio::sync::mpsc::Sender;
use tracing::{debug, error, instrument, trace};

use crate::connection::{Connection, ConnectionHandle};
use crate::connections_manager::Error;
use crate::messages::{HttpRequest, Id, ProtoMessage};

/// Collection of connections between the device and the bridge.
pub(crate) struct Connections {
    /// Collection mapping every Connection ID with the corresponding [`tokio task`](tokio::task) spawned to
    /// handle it.
    connections: HashMap<Id, ConnectionHandle>,
    /// Write side of the channel used by each connection to send data to the [`ConnectionsManager`].
    /// This field is only cloned and passed to every connection when created.
    tx_ws: Sender<ProtoMessage>,
}

impl Debug for Connections {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connections")
            .field("connections", &self.connections.keys())
            .finish()
    }
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
                trace!("connection {id} replaced");
            }
            Entry::Vacant(entry) => {
                entry.insert(f()?);
                trace!("connection {id} inserted");
            }
        }

        Ok(())
    }

    /// Remove all terminated connection from the connections' collection.
    #[instrument(skip_all)]
    pub(crate) fn remove_terminated(&mut self) {
        trace!("removing terminated connections");
        self.connections
            .retain(|_k, con_handle| !con_handle.is_finished());
        trace!("terminated connections removed");
    }

    /// Terminate all connections, both active and non.
    pub(crate) fn disconnect(&mut self) {
        self.connections.values_mut().for_each(|con| con.abort());
        self.connections.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Barrier;

    fn create_con_handle<F>(f: F) -> Result<ConnectionHandle, Error>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Ok(ConnectionHandle {
            handle: tokio::spawn(f),
        })
    }

    #[tokio::test]
    async fn test_try_add() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<ProtoMessage>(50);
        let mut collection = Connections::new(tx);

        let id = Id::try_from(b"test_id".to_vec()).unwrap();

        // use a barrier to wait one join handle task to be finished
        let barrier = Arc::new(Barrier::new(2));
        let barrier_cl = Arc::clone(&barrier);

        // add a new connection
        let res = collection.try_add(id.clone(), || {
            create_con_handle(async move {
                barrier_cl.wait().await;
            })
        });

        barrier.wait().await;
        assert!(res.is_ok());

        // add a connection with the same ID of an ended task
        let res = collection.try_add(id.clone(), || {
            create_con_handle(tokio::time::sleep(Duration::from_secs(10)))
        });
        assert!(res.is_ok(), "but got error {}", res.unwrap_err());

        // add a new connection with the same ID of an existing and non-terminated one.
        // this results in an error
        let res = collection.try_add(id.clone(), || create_con_handle(futures::future::ready(())));

        assert!(res.is_err());
    }
}
