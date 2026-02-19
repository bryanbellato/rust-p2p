use crate::client::Client;
use crate::server::Server;

use std::io::{self, BufRead};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::thread;
use threadpool::ThreadPool;

pub type SharedHosts = Arc<Mutex<Vec<(String, u16)>>>;

pub struct PeerToPeer {
    pub known_hosts: SharedHosts,
    pub port: u16,
}

impl PeerToPeer {
    pub fn new(port: u16) -> Self {
        PeerToPeer {
            known_hosts: Arc::new(Mutex::new(Vec::new())),
            port,
        }
    }

    pub fn run(&self) {
        let known_hosts = Arc::clone(&self.known_hosts);
        let port = self.port;

        thread::spawn(move || {
            if let Err(e) = server_function(known_hosts, port) {
                eprintln!("[SERVER] Fatal error: {}", e);
            }
        });

        client_function(Arc::clone(&self.known_hosts), self.port);
    }
}

fn register_host_if_new(known_hosts: &SharedHosts, ip: String, port: u16) {
    let mut hosts = known_hosts.lock().unwrap();
    if !hosts.iter().any(|(h, p)| h == &ip && *p == port) {
        println!("[SERVER] New host discovered: {}:{}", ip, port);
        hosts.push((ip, port));
    }
}

fn server_function(known_hosts: SharedHosts, port: u16) -> io::Result<()> {
    let server = Server::new(Ipv4Addr::new(0, 0, 0, 0), port)?;
    let pool = ThreadPool::new(50);

    for stream in server.listener.incoming() {
        match stream {
            Ok(stream) => {
                let hosts = Arc::clone(&known_hosts);
                pool.execute(move || handle_client(stream, hosts));
            }
            Err(e) => {
                eprintln!("[SERVER] Accept error: {}", e);
            }
        }
    }
    Ok(())
}

fn handle_client(mut stream: std::net::TcpStream, known_hosts: SharedHosts) {
    let client_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "Unknown".to_string());

    use std::io::{Read, Write};
    let mut buffer = [0u8; 4096];

    match stream.read(&mut buffer) {
        Ok(0) => (),
        Ok(n) => {
            let message_full = String::from_utf8_lossy(&buffer[..n]);

            if let Some((port_str, msg)) = message_full.split_once(':') {
                if let Ok(peer_listen_port) = port_str.parse::<u16>() {
                    register_host_if_new(&known_hosts, client_ip.clone(), peer_listen_port);
                    println!("[SERVER] Received from {}: {}", client_ip, msg);
                }
            } else {
                println!("[SERVER] Received malformed message from {}", client_ip);
            }

            let _ = stream.write_all(b"Message received");
        }
        Err(e) => eprintln!("[SERVER] Read error from {}: {}", client_ip, e),
    }
}

fn client_function(known_hosts: SharedHosts, my_port: u16) {
    let stdin = io::stdin();
    println!("[CLIENT] Enter messages to broadcast (Ctrl-D to quit):");

    for line in stdin.lock().lines() {
        let message = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if message.is_empty() {
            continue;
        }

        let peers: Vec<(String, u16)> = known_hosts.lock().unwrap().clone();

        if peers.is_empty() {
            println!("[CLIENT] No known peers yet.");
            continue;
        }

        let formatted_message = format!("{}:{}", my_port, message);

        for (ip, peer_port) in &peers {
            let client = Client::new(*peer_port);

            match ip.parse::<Ipv4Addr>() {
                Ok(addr) => match client.request(addr, &formatted_message) {
                    Ok(response) => {
                        println!("[CLIENT] Response from {}:{}: {}", ip, peer_port, response)
                    }
                    Err(e) => eprintln!("[CLIENT] Failed to reach {}:{}: {}", ip, peer_port, e),
                },
                Err(_) => eprintln!("[CLIENT] Invalid IP in known_hosts: {}", ip),
            }
        }
    }
}
