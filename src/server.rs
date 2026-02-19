use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};

pub struct Server {
    pub listener: TcpListener,
    pub port: u16,
}

impl Server {
    pub fn new(interface: Ipv4Addr, port: u16) -> io::Result<Self> {
        let address = SocketAddrV4::new(interface, port);
        let listener = TcpListener::bind(address)?;
        println!("[SERVER] Listening on port {}", port);
        Ok(Server { listener, port })
    }
}
