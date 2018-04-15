extern crate mio;
extern crate http_muncher;

use http_muncher::{Parser, ParserHandler};
use mio::*;
use mio::net::*;
use std::time::Duration;

use std::collections::HashMap;
use std::io::{Read, ErrorKind};
use std::io;

struct HttpParser;
impl ParserHandler for HttpParser {

}

impl HttpParser {
    fn new() -> HttpParser {
        HttpParser { }
    }
}

struct WebSocketClient {
    socket: TcpStream,
    http_parser: Parser,
}

impl WebSocketClient {
    pub fn new(socket: TcpStream) -> WebSocketClient {
        WebSocketClient {
            socket: socket,
            http_parser: Parser::request()
        }
    }

    pub fn read(&mut self) {
        loop {
            let mut buf = [0; 2048];
            match self.socket.read(&mut buf) {
                Err(e) => {
                    match e.kind() {
                        ErrorKind::Interrupted => {
                            println!("Read interrupted, try again");
                        },
                        ErrorKind::WouldBlock => {
                            println!("Socket read would block, returning");
                            return;
                        },
                        _ => {
                            println!("Read socket error. {:?}", e.kind());
                            return;
                        }
                    }
                },
                Ok(0) => {
                    println!("Socket has no more bytes to read");
                    break;
                },
                Ok(len) => {
                    println!("Bytes read: {}\nRead {:?}", len, std::str::from_utf8(&buf[0..len]));
                    self.http_parser.parse(&mut HttpParser::new(), &buf[0..len]);
                    if self.http_parser.is_upgrade() {
                        println!("Is HTTP upgrade");
                        break;
                    }
                }
            }
        }
    }
}

impl Evented for WebSocketClient  {
    fn register(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt)
        -> io::Result<()>
    {
        // Delegate the `register` call to `socket`
        poll.register(&self.socket, token, interest, opts)
    }

    fn reregister(&self, poll: &Poll, token: Token, interest: Ready, opts: PollOpt)
        -> io::Result<()>
    {
        // Delegate the `reregister` call to `socket`
        poll.reregister(&self.socket, token, interest, opts)
    }

    fn deregister(&self, poll: &Poll) -> io::Result<()> {
        // Delegate the `deregister` call to `socket`
        poll.deregister(&self.socket)
    }}

const SERVER: Token = Token(0);

struct WebSocketServer {
    socket: TcpListener,
    clients: HashMap<Token, WebSocketClient>, // Use `slab` maybe?
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
            Ok((sock, _addr)) => {
                WebSocketClient::new(sock)
            }
        };

        self.token_counter += 1;
        let new_token = Token(self.token_counter);

        poll.register(&client, new_token, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
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
                token => {
                    println!("Received event from {:?}", token);
                    match websocket.clients.get_mut(&token) {
                        None => {
                            println!("No client represents this token");
                        },
                        Some(client) => {
                            client.read();
                            client.reregister(&poll, token, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
                        }
                    }
                }
            }
        }
    }
}
