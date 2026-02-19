## Peer to Peer Network in Rust

This is a simple decentralized network with a messaging feature. It allows multiple instances of the program to connect, discover each other, and exchange messages without a central server.
It is intended for learning on how a peer-to-peer network works.

The server runs in a background thread, listening for incoming TCP connections. When it receives a message, it extracts the sender's listening port and adds them to its known_hosts list.
The client runs in the main thread. it takes a input and broadcasts that message to every node currently in its known_hosts list.

To join the network, a new node needs to know at least **one** address to start with.
The first node is the seed, without a bootstrap address.
The second node point it to the first node using the `--bootstrap` parameter on the CLI.
When the second node sends a message to the first, the first node learns about the second node's existence and saves its IP and port.

---

### How to build the project?
Install Rust in your OS and build it with cargo.
```
cargo build --release
```
---

### Usage
After building the project, you can execute `cargo run` or `./p2p` located on `./target/debug/p2p`.

**You have the following arguments options:**
```
--port <number>
```
The port this node will listen on for incoming messages. (Default: 1248)
```
--bootstrap <IP:PORT>
```
The address of an already-running node to connect to and join the network.

The first node starts by default on port `1248` and has no one to talk to yet.
```
cargo run -- --port <port number>
```

This second node connects to the first node to join the network, bootstrapping it.
```
cargo run -- --port <port number> --bootstrap <IP>:<PORT>
```

You can also start a third node bootstrapping to the second node IP:PORT.

---
## Message Protocol
Messages are sent as plain text over TCP with a simple header (`PORT:MESSAGE_CONTENT`)

When a node receives this, it parses the `PORT` to know where the sender is listening. Then, it registers the sender's IP and Port in its internal directory (`known_hosts`).  Finally, it prints the `MESSAGE_CONTENT` to the console.
