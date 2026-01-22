// Copyright 2024 SECO Mind Srl
// SPDX-License-Identifier: Apache-2.0

//! Collection of connections and respective methods.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::{Debug, Formatter};

use tokio::sync::mpsc::Sender;
use tracing::{debug, error, instrument, trace};

use crate::connection::{Connection, ConnectionHandle};
use crate::connections_manager::Error;
use crate::messages::{
    Http as ProtoHttp, HttpRequest, Id, ProtoMessage, WebSocket as ProtoWebSocket,
};

/// Connections' collection between the device and Edgehog.
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

    /// Handle the reception of an HTTP proto message from Edgehog.
    #[instrument(skip_all)]
    pub(crate) fn handle_http(&mut self, http: ProtoHttp) -> Result<(), Error> {
        let ProtoHttp {
            request_id,
            http_msg,
        } = http;

        // the HTTP message can't be an http response
        let Some(http_req) = http_msg.into_req() else {
            error!("Http response should not be sent by Edgehog");
            return Err(Error::WrongMessage(request_id));
        };

        // before executing the HTTP request, check if it is an Upgrade request.
        // if so, handle it properly.
        if http_req.is_ws_upgrade() {
            debug!("Upgrade the HTTP connection to WS");
            return self.add_ws(request_id, http_req);
        }

        let tx_ws = self.tx_ws.clone();

        self.try_add(request_id.clone(), || {
            Connection::with_http(request_id, tx_ws, http_req).map_err(Error::from)
        })
    }

    /// Create a new WebSocket [`Connection`].
    #[instrument(skip(self))]
    fn add_ws(&mut self, request_id: Id, http_req: HttpRequest) -> Result<(), Error> {
        debug_assert!(http_req.is_ws_upgrade());

        let tx_ws = self.tx_ws.clone();

        self.try_add(request_id.clone(), || {
            Connection::with_ws(request_id, tx_ws, http_req).map_err(Error::from)
        })
    }

    /// Handle the reception of a WebSocket protocol message from Edgehog.
    #[instrument(skip(self, ws))]
    pub(crate) async fn handle_ws(&mut self, ws: ProtoWebSocket) -> Result<(), Error> {
        let ProtoWebSocket { socket_id, message } = ws;

        // check if there exist a WebSocket connection with the specified id
        // and send a WebSocket message toward the task responsible for handling it
        match self.connections.entry(socket_id.clone()) {
            Entry::Occupied(entry) => {
                let handle = entry.get();
                let proto_msg = ProtoMessage::WebSocket(ProtoWebSocket {
                    socket_id: socket_id.clone(),
                    message,
                });
                handle.send(proto_msg).await.map_err(Error::from)
            }
            Entry::Vacant(_entry) => {
                error!("WebSocket connection {socket_id} not found");
                Err(Error::ConnectionNotFound(socket_id))
            }
        }
    }

    /// Try add a new connection.
    #[instrument(skip(self, f))]
    pub(crate) fn try_add<F>(&mut self, id: Id, f: F) -> Result<(), Error>
    where
        F: FnOnce() -> Result<ConnectionHandle, Error>,
    {
        // check if there exist a connection with the specified id
        match self.connections.entry(id.clone()) {
            Entry::Occupied(mut entry) => {
                error!("entry already occupied");

                let handle = entry.get_mut();

                // check if the connection is finished or not. If it is finished, return the
                // entry so that a new connection with the same Key can be created
                if !handle.is_finished() {
                    return Err(Error::IdAlreadyUsed(id));
                }

                debug!("connection terminated, replacing it with a new connection");
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

    /// Terminate all connections.
    pub(crate) fn disconnect(&mut self) {
        self.connections.values_mut().for_each(|con| con.abort());
        self.connections.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::WriteHandle;
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
            connection: WriteHandle::Http,
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
