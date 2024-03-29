<!---
  Copyright 2024 SECO Mind Srl
  SPDX-License-Identifier: Apache-2.0
-->

# Edgehog Device Runtime Forwarder Library

## Overview

The Edgehog Device Runtime Forwarder Library facilitates the communication and interaction
between devices and Edgehog through WebSocket connections. This could be useful in the scenario where a remote user, who
has access to Edgehog, wishes to open a remote terminal exposed by the device on a specific port and make it
accessible through a WebSocket connection.
This README provides a guide to  understanding the structure of the library and how its components work together.

## Sequence Diagram

```mermaid
sequenceDiagram
  actor User
  participant Edgehog
  participant Astarte
  participant Device Runtime
  participant Forwarder
  participant Service (ttyd)

  User ->> Edgehog: Open remote terminal
  Edgehog ->> Astarte: Remote terminal request
  Astarte ->> Device Runtime:  Remote terminal request
  Device Runtime ->> Forwarder: Session info (host, port, session_token)
  Forwarder ->> Edgehog: Start WebSocket session
  Edgehog ->> Forwarder: Connect to Service (ttyd)
  Forwarder ->> Service (ttyd): Start WebSocket connection

  Note over Service (ttyd),User: Remote terminal connection
```

The end user triggers a remote terminal request on the Edgehog Device Manager interface. Edgehog is responsible for
sending the request to Astarte, which then forwards it to the Device Runtime, handling device operations.
The Device Runtime retrieves session information from the remote terminal request and communicates it to the Forwarder,
a module capable of managing connections with the device.
Using the received information, the Forwarder establishes a WebSocket connection with Edgehog. On top of this
connection, other types of connections (HTTP, WebSocket, etc.) can be established. A specific use case is the opening
of a remote terminal on the device. In this scenario, Edgehog requests the Forwarder to initiate a connection with a
service called ttyd, exposing a terminal on a specified port through a WebSocket connection. Consequently, through
a web page, the end user gains access to a terminal.

## Components

### Connections Manager

- Establishes a WebSocket connection between the device and Edgehog.
- Acts as a multiplexer for forwarding messages between Edgehog and internal device connections (e.g., with
[ttyd](https://github.com/tsl0922/ttyd)).
- Handles WebSocket frames (carrying HTTP requests/responses or other WebSocket frames), decoding binary data into an
  internal Rust representation.
- Handles reconnection operations on WebSocket errors.
- Maintains a collection of connections with channel writing handles.

### Collection

- Maintains a map of connections, each identified by a Host, a port and a session token.
- Defines methods to add and remove new device connections.
- Allows disconnecting from all connections.

### Connection

- Defines `Connection` and `ConnectionHandle` structs, necessary to create new connections. At the moment, the only
supported connections types are HTTP and WebSocket.
- Spawns Tokio tasks responsible for managing connections.
- Sends messages and encodes responses into protobuf messages.

### Messages

- Defines an internal representation of the possible messages travelling on the WebSocket connection with Edgehog.
- Implement utility methods and traits to perform simple type conversions.

### Astarte

- Provides the necessary functions to convert an `Astarte aggregate object` into a `SessionInfo` struct, used by the
  library to store the session information between the device and Edgehog.

## Usage

The entry point to the library's functionalities is the `ConnectionsManager` struct, which exposes two main methods:

- **`connect`**: This method takes as input the URL containing the information necessary to establish a
  WebSocket session with Edgehog.
- **`handle_connections`**: This method continuously loops, managing send/receive events over the WebSocket channel.

The `astarte.rs` module contains the functionalities that can be utilized to retrieve
the URL from the session information sent by Astarte.

## WebSocket Secure

The establishment of a secure WebSocket connection with Edgehog is automatically performed based on a boolean value sent
by Astarte to the device. If the flag `secure` is set, the device will load the native device certificates and store
them in the root certificate store.
It is also possible to set the environment variable `EDGEHOG_FORWARDER_CA_PATH` to install a custom CA certificate in
the root certificate store, which is useful in the case of testing on localhost.