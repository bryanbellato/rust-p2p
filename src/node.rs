use crate::client::Client;
use crate::server::Server;
use rust_blockchain::{
    blockchain::{Blockchain, SharedBlockchain},
    keypair::KeyPair,
    transaction::Transaction,
    Message,
};
use std::io::{self, BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use threadpool::ThreadPool;

pub type SharedHosts = Arc<Mutex<Vec<(String, u16)>>>;

pub type SharedMempool = Arc<RwLock<Vec<Transaction>>>;

pub struct Node {
    pub blockchain: SharedBlockchain,
    pub mempool: SharedMempool,
    pub known_hosts: SharedHosts,
    pub keypair: KeyPair,
    pub port: u16,
}

impl Node {
    pub fn new(port: u16, keypair: KeyPair, blockchain: Blockchain) -> Self {
        Node {
            blockchain: rust_blockchain::blockchain::new_shared(blockchain),
            mempool: Arc::new(RwLock::new(Vec::new())),
            known_hosts: Arc::new(Mutex::new(Vec::new())),
            keypair,
            port,
        }
    }

    pub fn run(&self) {
        let blockchain = Arc::clone(&self.blockchain);
        let mempool = Arc::clone(&self.mempool);
        let known_hosts = Arc::clone(&self.known_hosts);
        let port = self.port;

        thread::spawn(move || {
            if let Err(e) = server_thread(blockchain, mempool, known_hosts, port) {
                eprintln!("[SERVER] Fatal error: {}", e);
            }
        });

        // give the server thread a moment to bind before we start sending.
        thread::sleep(std::time::Duration::from_millis(100));

        // if we bootstrapped with known peers,
        // we adopt the network's chain before accepting user input
        startup_sync(&self.known_hosts, &self.blockchain, self.port);

        client_loop(
            Arc::clone(&self.known_hosts),
            Arc::clone(&self.blockchain),
            Arc::clone(&self.mempool),
            self.port,
        );
    }
}

// serialise a message to a JSON string
pub fn encode(msg: &Message) -> io::Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    bytes.push(b'\n');
    Ok(bytes)
}

// deserialise a JSON string
pub fn decode(data: &[u8]) -> io::Result<Message> {
    serde_json::from_slice(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn server_thread(
    blockchain: SharedBlockchain,
    mempool: SharedMempool,
    known_hosts: SharedHosts,
    port: u16,
) -> io::Result<()> {
    let server = Server::new(Ipv4Addr::new(0, 0, 0, 0), port)?;
    let pool = ThreadPool::new(50);

    for stream in server.listener.incoming() {
        match stream {
            Ok(stream) => {
                let bc = Arc::clone(&blockchain);
                let mp = Arc::clone(&mempool);
                let hosts = Arc::clone(&known_hosts);
                pool.execute(move || handle_stream(stream, bc, mp, hosts));
            }
            Err(e) => eprintln!("[SERVER] Accept error: {}", e),
        }
    }
    Ok(())
}

fn handle_stream(
    stream: std::net::TcpStream,
    blockchain: SharedBlockchain,
    mempool: SharedMempool,
    known_hosts: SharedHosts,
) {
    let client_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[SERVER] Clone error: {}", e);
            return;
        }
    };

    let reader = BufReader::new(stream);

    for line in reader.lines() {
        let line = match line {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => continue,
            Err(e) => {
                eprintln!("[SERVER] Read error from {}: {}", client_ip, e);
                return;
            }
        };

        match decode(line.as_bytes()) {
            Ok(msg) => {
                let response = handle_message(msg, &client_ip, &blockchain, &mempool, &known_hosts);
                if let Ok(bytes) = encode(&response) {
                    let _ = writer.write_all(&bytes);
                }
            }
            Err(e) => eprintln!("[SERVER] Malformed message from {}: {}", client_ip, e),
        }
    }
}

fn handle_message(
    msg: Message,
    client_ip: &str,
    blockchain: &SharedBlockchain,
    mempool: &SharedMempool,
    known_hosts: &SharedHosts,
) -> Message {
    match msg {
        Message::Ping { port } => {
            println!("[SERVER] Ping from {}:{}", client_ip, port);
            register_host(known_hosts, client_ip.to_string(), port);
            Message::Pong { port: 0 } // wip: replace 0 with this node's own port
        }

        Message::Pong { port } => {
            println!("[SERVER] Pong from {}:{}", client_ip, port);
            Message::Pong { port: 0 }
        }

        Message::NewBlock(block) => {
            println!(
                "[SERVER] NewBlock from {}: hash={}",
                client_ip,
                block.hash()
            );

            let mut bc = blockchain.write().unwrap();
            let mut chain = bc.chain().to_vec();
            chain.push(block);
            if bc.replace_chain(chain) {
                println!("[SERVER] Chain updated from peer {}", client_ip);
            }
            Message::Pong { port: 0 } // ack
        }

        Message::RequestBlock { index } => {
            println!("[SERVER] RequestBlock {} from {}", index, client_ip);
            let bc = blockchain.read().unwrap();
            match bc.chain().get(index) {
                Some(block) => Message::Block(block.clone()),
                None => {
                    eprintln!("[SERVER] Block {} not found", index);
                    Message::Pong { port: 0 }
                }
            }
        }

        Message::Block(block) => {
            println!(
                "[SERVER] Block received from {}: hash={}",
                client_ip,
                block.hash()
            );
            Message::Pong { port: 0 }
        }

        Message::NewTransaction(tx) => {
            println!("[SERVER] NewTransaction from {}: id={}", client_ip, tx.id());
            let mut mp = mempool.write().unwrap();
            if !mp.iter().any(|t| t.id() == tx.id()) {
                mp.push(tx);
                println!("[SERVER] Transaction added to mempool (size={})", mp.len());
            }
            Message::Pong { port: 0 }
        }

        Message::RequestChain => {
            println!("[SERVER] RequestChain from {}", client_ip);
            let bc = blockchain.read().unwrap();
            let blocks = bc.chain().to_vec();
            let length = blocks.len();
            Message::ChainResponse { blocks, length }
        }

        Message::ChainResponse { blocks, length } => {
            println!(
                "[SERVER] ChainResponse from {}: {} blocks",
                client_ip, length
            );
            let mut bc = blockchain.write().unwrap();
            if bc.replace_chain(blocks) {
                println!("[SERVER] Chain replaced with peer's longer chain");
            }
            Message::Pong { port: 0 }
        }

        Message::RequestInventory => {
            println!("[SERVER] RequestInventory from {}", client_ip);
            let bc = blockchain.read().unwrap();
            let block_hashes = bc.chain().iter().map(|b| b.hash().to_string()).collect();
            Message::Inventory { block_hashes }
        }

        Message::Inventory { block_hashes } => {
            println!(
                "[SERVER] Inventory from {}: {} hashes",
                client_ip,
                block_hashes.len()
            );
            let bc = blockchain.read().unwrap();
            let known: std::collections::HashSet<_> =
                bc.chain().iter().map(|b| b.hash().to_string()).collect();
            drop(bc);

            for (index, hash) in block_hashes.iter().enumerate() {
                if !known.contains(hash) {
                    println!("[SERVER] Missing block at index {}: {}", index, hash);
                }
            }
            Message::Pong { port: 0 }
        }
    }
}

fn register_host(known_hosts: &SharedHosts, ip: String, port: u16) {
    let mut hosts = known_hosts.lock().unwrap();
    if !hosts.iter().any(|(h, p)| h == &ip && *p == port) {
        println!("[NODE] New peer discovered: {}:{}", ip, port);
        hosts.push((ip, port));
    }
}

fn startup_sync(known_hosts: &SharedHosts, blockchain: &SharedBlockchain, my_port: u16) {
    let peers: Vec<(String, u16)> = known_hosts.lock().unwrap().clone();

    if peers.is_empty() {
        println!("[SYNC] No peers to sync with — this node is the network origin.");
        return;
    }

    println!(
        "[SYNC] Starting up — syncing chain with {} known peer(s)...",
        peers.len()
    );

    for (ip, peer_port) in &peers {
        match ip.parse::<Ipv4Addr>() {
            Ok(addr) => {
                // announce our listening port so the peer can register us
                let ping = Message::Ping { port: my_port };
                if let Some(reply) = send_to_peer(addr, *peer_port, &ping) {
                    println!("[SYNC] Ping → {}:{} got {:?}", ip, peer_port, reply);
                }

                // request their chain; the response handler will call
                let req = Message::RequestChain;
                if let Some(reply) = send_to_peer(addr, *peer_port, &req) {
                    if let Message::ChainResponse { blocks, length } = reply {
                        println!(
                            "[SYNC] Got chain from {}:{} with {} block(s)",
                            ip, peer_port, length
                        );
                        let mut bc = blockchain.write().unwrap();
                        if bc.replace_chain_bootstrap(blocks) {
                            println!(
                                "[SYNC] Adopted network chain — genesis: {}...",
                                &bc.chain()[0].hash()[..16]
                            );
                        } else {
                            println!("[SYNC] Our chain is already up to date.");
                        }
                    }
                }
            }
            Err(_) => eprintln!("[SYNC] Invalid peer IP: {}", ip),
        }
    }
}

// send a single message to a specific peer and return the decoded response
// returns None on any network or encoding error
fn send_to_peer(addr: Ipv4Addr, port: u16, msg: &Message) -> Option<Message> {
    let payload = encode(msg).ok()?;
    let payload_str = String::from_utf8_lossy(&payload).to_string();
    let client = Client::new(port);
    match client.request(addr, &payload_str) {
        Ok(response) => decode(response.trim_end().as_bytes()).ok(),
        Err(e) => {
            eprintln!("[SYNC] ✗ {}:{} — {}", addr, port, e);
            None
        }
    }
}

fn client_loop(
    known_hosts: SharedHosts,
    blockchain: SharedBlockchain,
    mempool: SharedMempool,
    my_port: u16,
) {
    let stdin = io::stdin();
    println!("[CLIENT] Ready. Commands: ping | chain | inventory | mempool | quit");

    for line in stdin.lock().lines() {
        let input = match line {
            Ok(l) => l.trim().to_string(),
            Err(_) => break,
        };
        if input.is_empty() {
            continue;
        }

        match input.as_str() {
            "quit" => break,

            "mempool" => {
                let mp = mempool.read().unwrap();
                println!("[CLIENT] Mempool: {} pending transactions", mp.len());
                for tx in mp.iter() {
                    println!(
                        "  {} → {} : {} coins",
                        tx.from_address(),
                        tx.to_address(),
                        tx.amount()
                    );
                }
            }

            "chain" => {
                let bc = blockchain.read().unwrap();
                println!(
                    "[CLIENT] Local chain: {} block(s), tip={}",
                    bc.chain().len(),
                    bc.chain().last().map(|b| &b.hash()[..12]).unwrap_or("none")
                );
                drop(bc);
                broadcast(&known_hosts, &Message::RequestChain, my_port);
            }

            "inventory" => {
                broadcast(&known_hosts, &Message::RequestInventory, my_port);
            }

            "ping" => {
                broadcast(&known_hosts, &Message::Ping { port: my_port }, my_port);
            }

            other => {
                println!(
                    "[CLIENT] Unknown command '{}'. Try: ping | chain | inventory | mempool | quit",
                    other
                );
            }
        }
    }
}

// encode msg and send it to every known peer, printing the decoded response
fn broadcast(known_hosts: &SharedHosts, msg: &Message, my_port: u16) {
    let peers: Vec<(String, u16)> = known_hosts.lock().unwrap().clone();

    if peers.is_empty() {
        println!("[CLIENT] No known peers.");
        return;
    }

    let payload = match encode(msg) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[CLIENT] Encode error: {}", e);
            return;
        }
    };
    let payload_str = String::from_utf8_lossy(&payload).to_string();

    for (ip, peer_port) in &peers {
        let client = Client::new(*peer_port);
        match ip.parse::<Ipv4Addr>() {
            Ok(addr) => match client.request(addr, &payload_str) {
                Ok(response) => match decode(response.trim_end().as_bytes()) {
                    Ok(reply) => println!("[CLIENT] ← {}:{} {:?}", ip, peer_port, reply),
                    Err(_) => println!("[CLIENT] ← {}:{} (raw) {}", ip, peer_port, response),
                },
                Err(e) => eprintln!("[CLIENT] ✗ {}:{} {}", ip, peer_port, e),
            },
            Err(_) => eprintln!("[CLIENT] Invalid IP: {}", ip),
        }
    }
    let _ = my_port;
}
