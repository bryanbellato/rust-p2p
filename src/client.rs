use std::io::{self, Read, Write};
use std::net::TcpStream;

pub struct Client {
    pub port: u16,
}

impl Client {
    pub fn new(port: u16) -> Self {
        Client { port }
    }

    pub fn request(&self, host: &str, message: &str) -> io::Result<String> {
        let mut stream = TcpStream::connect((host, self.port))?;
        stream.write_all(message.as_bytes())?;
        let mut buffer = vec![0u8; 30_000];
        let bytes_read = stream.read(&mut buffer)?;

        Ok(String::from_utf8_lossy(&buffer[..bytes_read]).to_string())
    }
}