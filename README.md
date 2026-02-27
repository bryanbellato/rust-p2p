## (Blockchain) Peer to Peer Network in Rust

This is a simple decentralized network with a blockchain feature, based on the original Bitcoin style. It allows multiple instances of the program to connect, discover each other, and interact with common blockchain features (such as mining, transactions, et cetera).
It is intended for learning on how a peer-to-peer and a blockchain network works.

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

## Usage
After building the project, you can execute `cargo run` or `./p2p` located on `./target/debug/p2p`.

**You have the following arguments options:**
### Connection
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


### Blockchain Features
`whoami`
Prints your node's public key (your address) and your current confirmed balance.

`send <to_address> <amount>`
Creates, signs, and broadcasts a transaction to the network.
`<to_address>` is the full 130-char public key of the recipient. Get it from them running `whoami` on their node.
The fee is calculated automatically from the transaction size and the network's minimum fee rate. You don't specify it.
The transaction lands in your local mempool immediately and is broadcast to peers. **It becomes confirmed once someone mines a block containing it.**

`mine`
Mines a new block containing everything currently in the mempool, claims the block reward + all transaction fees, and broadcasts the block to all peers.
If another node mines and broadcasts a block before yours finishes, your work is discarded (stale block detection).

`mempool`
Shows all transactions currently waiting to be mined on this node.

`chain`
Shows your local chain height and tip hash, then requests the latest chain from all peers (triggering a sync if they're ahead of you).

`inventory`
Asks all peers for a list of their block hashes without downloading the full blocks. Useful for quickly checking whether your chain is in sync without the bandwidth cost of a full chain request.

`ping`
Sends a Ping to all known peers. Each peer registers your listening port and replies with their own. Useful for checking connectivity and for mutual peer discovery.

`quit`
I don't need to explain this one lol.