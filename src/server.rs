use std::io;
use std::net::TcpListener;

pub struct Server {
    pub listener: TcpListener,
    pub port: u16,
}

impl Server {
    pub fn new(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(format!("[::]:{}", port))
            .or_else(|_| TcpListener::bind(format!("0.0.0.0:{}", port)))?;
            
        println!("[SERVER] Listening on port {}", port);
        Ok(Server { listener, port })
    }
}