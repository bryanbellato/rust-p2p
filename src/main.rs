use p2p::peer_to_peer::PeerToPeer;
use std::env;
use std::net::SocketAddr;

fn main() {
    let args: Vec<String> = env::args().collect();

    let mut port = 1248;
    let mut bootstrap: Option<SocketAddr> = None;

    // manual parsing
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
                    bootstrap = Some(b.parse().expect("Invalid bootstrap address (use IP:PORT)"));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let node = PeerToPeer::new(port);

    if let Some(addr) = bootstrap {
        let mut hosts = node.known_hosts.lock().unwrap();
        hosts.push((addr.ip().to_string(), addr.port()));
        println!("[CLIENT] Bootstrapping with: {}", addr);
    }

    node.run();
}
