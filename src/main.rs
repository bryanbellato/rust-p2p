use p2p::node::Node;
use rust_blockchain::{blockchain::Blockchain, keypair::KeyPair};
use std::env;
use std::net::SocketAddr;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut port: u16 = 1248;
    let mut bootstrap: Option<SocketAddr> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                if let Some(p) = args.get(i + 1) {
                    port = p.parse().expect("Invalid port number");
                    i += 1;
                }
            }
            "--bootstrap" => {
                if let Some(b) = args.get(i + 1) {
                    use std::net::ToSocketAddrs;
                    let addr = b.to_socket_addrs()
                        .expect("Failed to resolve bootstrap address")
                        .next()
                        .expect("No address found");
                    bootstrap = Some(addr);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let keypair = KeyPair::generate().expect("Failed to generate keypair");
    let miner_address = keypair.get_public_key().to_string();
    println!("[NODE] Identity: {}...", &miner_address[..20]);

    // difficulty=5, reward=50 coins, min_fee_rate=10 sat/byte
    let blockchain = Blockchain::new(5, 50.0, 10, miner_address.clone());

    let node = Node::new(port, keypair, blockchain);

    // optionally bootstrap from a known peer.
    if let Some(addr) = bootstrap {
        let mut hosts = node.known_hosts.lock().unwrap();
        hosts.push((addr.ip().to_string(), addr.port()));
        println!("[NODE] Bootstrapping from: {}", addr);
    }

    node.run();
}
