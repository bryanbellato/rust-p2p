use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};

pub struct Client {
    pub port: u16,
}

impl Client {
    pub fn new(port: u16) -> Self {
        Client { port }
    }

    pub fn request(&self, server_ip: Ipv4Addr, message: &str) -> io::Result<String> {
        let address = SocketAddrV4::new(server_ip, self.port);
        let mut stream = TcpStream::connect(address)?;

        stream.write_all(message.as_bytes())?;

        let mut buffer = vec![0u8; 30_000];
        let bytes_read = stream.read(&mut buffer)?;

        Ok(String::from_utf8_lossy(&buffer[..bytes_read]).to_string())
    }
}
