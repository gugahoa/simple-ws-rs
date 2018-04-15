extern crate mio;
use mio::*;
use mio::net::*;
use std::time::Duration;

use std::collections::HashMap;

const SERVER: Token = Token(0);

struct WebSocketServer {
    socket: TcpListener,
    clients: HashMap<Token, TcpStream>, // Use `slab` maybe?
    token_counter: usize,
}

impl WebSocketServer {
    pub fn new(addr: &str) -> WebSocketServer {
        let bind_addr = addr.parse().unwrap();
        WebSocketServer {
            socket: TcpListener::bind(&bind_addr).unwrap(),
            clients: HashMap::new(),
            token_counter: 0,
        }
    }

    pub fn accept(&mut self, poll: &Poll) -> () {
        let client = match self.socket.accept() {
            Err(e) => {
                println!("Accept error: {}", e);
                return;
            },
            Ok((sock, addr)) => sock
        };

        self.token_counter += 1;
        let new_token = Token(self.token_counter);

        poll.register(&client, new_token, Ready::readable(), PollOpt::edge()).unwrap();
        self.clients.insert(new_token, client);
    }
}

fn main() {
    println!("Hello, world!");
    let poll = Poll::new().unwrap();

    let mut websocket = WebSocketServer::new("0.0.0.0:10000");
    poll.register(&websocket.socket, SERVER, Ready::readable(), PollOpt::edge()).unwrap();

    let mut events = Events::with_capacity(1024);
    loop {
        poll.poll(&mut events, Some(Duration::from_secs(5))).unwrap();

        for event in events.iter() {
            match event.token() {
                SERVER => {
                    println!("Received event from websocket");
                    let _ = websocket.accept(&poll);
                }
                Token(token) => {
                    println!("Received event from {}", token);
                }
            }
        }
    }
}
