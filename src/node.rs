use crate::client::Client;
use crate::server::Server;
use rust_blockchain::{
    blockchain::{Blockchain, SharedBlockchain},
    keypair::KeyPair,
    transaction::Transaction,
    Message,
};
use std::io::{self, BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub is_mining: Arc<AtomicBool>,
}

impl Node {
    pub fn new(port: u16, keypair: KeyPair, blockchain: Blockchain) -> Self {
        Node {
            blockchain: rust_blockchain::blockchain::new_shared(blockchain),
            mempool: Arc::new(RwLock::new(Vec::new())),
            known_hosts: Arc::new(Mutex::new(Vec::new())),
            keypair,
            port,
            is_mining: Arc::new(AtomicBool::new(false)),
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
            Arc::clone(&self.is_mining),
            self.keypair.clone(),
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
    let server = Server::new(port)?;
    let pool = ThreadPool::new(50);

    for stream in server.listener.incoming() {
        match stream {
            Ok(stream) => {
                let bc = Arc::clone(&blockchain);
                let mp = Arc::clone(&mempool);
                let hosts = Arc::clone(&known_hosts);
                pool.execute(move || handle_stream(stream, bc, mp, hosts, port));
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
    my_port: u16,
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
                let response = handle_message(
                    msg,
                    &client_ip,
                    &blockchain,
                    &mempool,
                    &known_hosts,
                    my_port,
                );
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
    my_port: u16,
) -> Message {
    match msg {
        Message::Ping { port } => {
            println!("[SERVER] Ping from {}:{}", client_ip, port);
            register_host(known_hosts, client_ip.to_string(), port);
            Message::Pong { port: my_port }
        }

        Message::Pong { port } => {
            if port != 0 {
                println!(
                    "[SERVER] Pong from {}:{} (their port: {})",
                    client_ip, port, port
                );
                register_host(known_hosts, client_ip.to_string(), port);
            }
            Message::Pong { port: my_port }
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
            Message::Pong { port: my_port }
        }

        Message::RequestBlock { index } => {
            println!("[SERVER] RequestBlock {} from {}", index, client_ip);
            let bc = blockchain.read().unwrap();
            match bc.chain().get(index) {
                Some(block) => Message::Block(block.clone()),
                None => {
                    eprintln!("[SERVER] Block {} not found", index);
                    Message::Pong { port: my_port }
                }
            }
        }

        Message::Block(block) => {
            println!(
                "[SERVER] Block received from {}: hash={}",
                client_ip,
                block.hash()
            );
            Message::Pong { port: my_port }
        }

        Message::NewTransaction(tx) => {
            println!("[SERVER] NewTransaction from {}: id={}", client_ip, tx.id());
            let mut mp = mempool.write().unwrap();
            if !mp.iter().any(|t| t.id() == tx.id()) {
                mp.push(tx);
                println!("[SERVER] Transaction added to mempool (size={})", mp.len());
            }
            Message::Pong { port: my_port }
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
            if bc.replace_chain_bootstrap(blocks) {
                println!("[SYNC] Adopted network chain ({} blocks)", length);
            }
            Message::Pong { port: my_port }
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
            Message::Pong { port: my_port }
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

    println!("[SYNC] Starting up — syncing chain with {} known peer(s)...", peers.len());

    for (ip, peer_port) in &peers {
        let ping = Message::Ping { port: my_port };
        if let Some(reply) = send_to_peer(ip, *peer_port, &ping) {
            println!("[SYNC] Ping → {}:{} got {:?}", ip, peer_port, reply);
        }

        let req = Message::RequestChain;
        if let Some(reply) = send_to_peer(ip, *peer_port, &req) {
            if let Message::ChainResponse { blocks, length } = reply {
                println!("[SYNC] Got chain from {}:{} with {} block(s)", ip, peer_port, length);
                let mut bc = blockchain.write().unwrap();
                if bc.replace_chain_bootstrap(blocks) {
                    println!("[SYNC] Adopted network chain — genesis: {}...", &bc.chain()[0].hash()[..16]);
                } else {
                    println!("[SYNC] Our chain is already up to date.");
                }
            }
        }
    }
}

// send a single message to a specific peer and return the decoded response
// returns None on any network or encoding error
fn send_to_peer(host: &str, port: u16, msg: &Message) -> Option<Message> {
    let payload = encode(msg).ok()?;
    let payload_str = String::from_utf8_lossy(&payload).to_string();
    let client = Client::new(port);
    
    match client.request(host, &payload_str) {
        Ok(response) => decode(response.trim_end().as_bytes()).ok(),
        Err(e) => {
            eprintln!("[SYNC] ERROR: {}:{} — {}", host, port, e);
            None
        }
    }
}

fn client_loop(
    known_hosts: SharedHosts,
    blockchain: SharedBlockchain,
    mempool: SharedMempool,
    my_port: u16,
    is_mining: Arc<AtomicBool>,
    keypair: KeyPair,
) {
    let stdin = io::stdin();
    println!("[CLIENT] Ready. Commands: send <to> <amount> | mine | whoami | ping | chain | inventory | mempool | quit");

    for line in stdin.lock().lines() {
        let input = match line {
            Ok(l) => l.trim().to_string(),
            Err(_) => break,
        };
        if input.is_empty() {
            continue;
        }

        let (cmd, rest) = match input.split_once(' ') {
            Some((c, r)) => (c, r.trim()),
            None         => (input.as_str(), ""),
        };

        match cmd {
            "quit" => break,

            "whoami" => {
                let addr = keypair.get_public_key();
                println!("[NODE] Your address: {}", addr);
                let bc = blockchain.read().unwrap();
                println!("[NODE] Balance: {} coins", bc.get_balance(addr).as_coins());
            }     
            

            "send" => {
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() != 2 {
                    println!("[SEND] Usage: send <to_address> <amount>");
                    continue;
                }
                let to_address  = parts[0];
                let amount: f64 = match parts[1].parse() {
                    Ok(v) => v,
                    Err(_) => { println!("[SEND] Invalid amount — use a decimal number e.g. 1.5"); continue; }
                };

                match create_and_broadcast_tx(
                    &blockchain, &mempool, &known_hosts,
                    &keypair, to_address, amount, my_port,
                ) {
                    Ok(tx_id) => println!("[SEND] Transaction broadcast: {}", tx_id),
                    Err(e)    => eprintln!("[SEND] Failed: {}", e),
                }
            }            

            "mine" => {
                if is_mining.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
                    println!("[MINE] Already mining — please wait.");
                    continue;
                }

                let bc          = Arc::clone(&blockchain);
                let mp          = Arc::clone(&mempool);
                let hosts       = Arc::clone(&known_hosts);
                let mining_flag = Arc::clone(&is_mining);
                let address     = keypair.get_public_key().to_string();
                let port        = my_port;

                thread::spawn(move || {
                    mine_block_async(bc, mp, hosts, mining_flag, address, port);
                });
            }

            "mempool" => {
                let mp = mempool.read().unwrap();
                println!("[CLIENT] Mempool: {} pending transaction(s)", mp.len());
                for tx in mp.iter() {
                    println!("  id={} | {}... → {}... | {} coins",
                             &tx.id()[..12],
                             &tx.from_address()[..16],
                             &tx.to_address()[..16],
                             tx.amount().as_coins());
                }
            }

            "chain" => {
                let bc = blockchain.read().unwrap();
                println!("[CLIENT] Local chain: {} block(s), tip={}...",
                         bc.chain().len(),
                         &bc.chain().last().map(|b| b.hash()).unwrap_or("none")[..16]);
                drop(bc);
                broadcast(&known_hosts, &Message::RequestChain, my_port);
            }

            "inventory" => {
                broadcast(&known_hosts, &Message::RequestInventory, my_port);
            }

            "ping" => {
                broadcast(&known_hosts, &Message::Ping { port: my_port }, my_port);
            }

            _ => {
                println!("[CLIENT] Unknown command '{}'. Try: send <to> <amount> | mine | whoami | ping | chain | inventory | mempool | quit", cmd);
            }
        }
    }
}

fn create_and_broadcast_tx(
    blockchain:  &SharedBlockchain,
    mempool:     &SharedMempool,
    known_hosts: &SharedHosts,
    keypair:     &KeyPair,
    to_address:  &str,
    amount:      f64,
    my_port:     u16,
) -> Result<String, String> {
    let from_address = keypair.get_public_key();

    let (available_balance, min_fee_rate) = {
        let bc = blockchain.read().unwrap();
        (bc.get_available_balance(from_address), bc.min_fee_rate())
    };

    let dummy = rust_blockchain::transaction::Transaction::new(from_address, to_address, amount, 0.0)
    .map_err(|e| format!("invalid transaction parameters: {}", e))?;

    let fee_satoshis = dummy.estimate_size() as u64 * min_fee_rate;
    let fee_coins    = fee_satoshis as f64 / 100_000_000.0;

    let total_cost = amount + fee_coins;
    if total_cost > available_balance.as_coins() {
        return Err(format!(
            "insufficient balance — need {:.8} coins (amount {:.8} + fee {:.8}), have {:.8}",
                           total_cost, amount, fee_coins, available_balance.as_coins()
        ));
    }

    let mut tx = rust_blockchain::transaction::Transaction::new(from_address, to_address, amount, fee_coins)
    .map_err(|e| format!("failed to create transaction: {}", e))?;

    tx.sign(keypair.get_private_key())
    .map_err(|e| format!("failed to sign transaction: {}", e))?;

    println!("[SEND] Created tx {} — amount: {:.8} coins, fee: {:.8} coins",
             &tx.id()[..16], amount, fee_coins);

    {
        let mut bc = blockchain.write().unwrap();
        bc.add_transaction(tx.clone())
        .map_err(|e| format!("transaction rejected by local node: {}", e))?;
    }

    {
        let mut mp = mempool.write().unwrap();
        mp.push(tx.clone());
    }

    let tx_id = tx.id().to_string();

    broadcast(known_hosts, &Message::NewTransaction(tx), my_port);

    Ok(tx_id)
}

fn mine_block_async(
    blockchain:    SharedBlockchain,
    mempool:       SharedMempool,
    known_hosts:   SharedHosts,
    is_mining:     Arc<AtomicBool>,
    miner_address: String,
    my_port:       u16,
) {
    struct MiningGuard(Arc<AtomicBool>);
    impl Drop for MiningGuard {
        fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); }
    }
    let _guard = MiningGuard(Arc::clone(&is_mining));

    println!("[MINE] Starting — draining mempool and preparing block...");

    let (mut candidate, difficulty) = {
        let mut bc = blockchain.write().unwrap();

        let pending: Vec<Transaction> = {
            let mut mp = mempool.write().unwrap();
            mp.drain(..).collect()
        };

        let mut accepted = 0usize;
        let mut rejected = 0usize;
        for tx in pending {
            match bc.add_transaction(tx) {
                Ok(_)  => accepted += 1,
                Err(e) => { eprintln!("[MINE] Skipping invalid tx: {}", e); rejected += 1; }
            }
        }
        println!("[MINE] Mempool drained — {} accepted, {} rejected", accepted, rejected);
        println!("[MINE] Pending in block: {} tx(s)", bc.pending_transactions().len());

        match bc.prepare_mining(miner_address) {
            Ok(result) => result,
            Err(e) => {
                eprintln!("[MINE] Failed to prepare block: {}", e);
                return;
            }
        }
    };

    println!("[MINE] Running PoW at difficulty {}...", difficulty);
    candidate.mine_block(difficulty);
    println!("[MINE] Block mined! hash={}...", &candidate.hash()[..20]);

    {
        let mut bc = blockchain.write().unwrap();
        match bc.commit_mined_block(candidate.clone()) {
            Ok(_) => println!("[MINE] Block committed — chain height: {}", bc.chain().len()),
            Err(e) => {
                eprintln!("[MINE] Commit failed (stale block?): {}", e);
                return;
            }
        }
    }

    println!("[MINE] Broadcasting NewBlock to peers...");
    broadcast(&known_hosts, &Message::NewBlock(candidate), my_port);
}

fn broadcast(known_hosts: &SharedHosts, msg: &Message, my_port: u16) {
    let peers: Vec<(String, u16)> = known_hosts.lock().unwrap().clone();

    if peers.is_empty() {
        println!("[CLIENT] No known peers.");
        return;
    }

    let payload = match encode(msg) {
        Ok(b) => b,
        Err(e) => { eprintln!("[CLIENT] Encode error: {}", e); return; }
    };
    let payload_str = String::from_utf8_lossy(&payload).to_string();

    for (ip, peer_port) in &peers {
        let client = Client::new(*peer_port);
        match client.request(ip, &payload_str) {
            Ok(response) => match decode(response.trim_end().as_bytes()) {
                Ok(Message::Pong { port }) if port != 0 => {
                    println!("[CLIENT] ← {}:{} Pong (their port: {})", ip, peer_port, port);
                    register_host(known_hosts, ip.clone(), port);
                }
                Ok(reply) => println!("[CLIENT] ← {}:{} {:?}", ip, peer_port, reply),
                Err(_)    => println!("[CLIENT] ← {}:{} (raw) {}", ip, peer_port, response),
            },
            Err(e) => eprintln!("[CLIENT] ERROR: {}:{} {}", ip, peer_port, e),
        }
    }
    let _ = my_port;
}