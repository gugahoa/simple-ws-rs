extern crate mio;
extern crate http_muncher;
extern crate base64;
extern crate sha1;
extern crate ws;

use http_muncher::{Parser, ParserHandler};
use mio::*;
use mio::unix::UnixReady;
use mio::net::*;
use std::time::Duration;

use std::collections::HashMap;
use std::io::{Read, ErrorKind, Write};
use std::io;
use std::fmt;

use ws::frame::WebSocketFrame;

fn gen_key(key: &String) -> String {
    let mut m = sha1::Sha1::new();

    m.update(key.as_bytes());
    m.update("258EAFA5-E914-47DA-95CA-C5AB0DC85B11".as_bytes());

    base64::encode(&m.digest().bytes())
}

#[derive(PartialEq)]
struct HttpParser {
    current_key: Option<String>,
    headers: HashMap<String, String>,
}

impl ParserHandler for HttpParser {
    fn on_header_field(&mut self, _parser: &mut Parser, s: &[u8]) -> bool {
        self.current_key = Some(std::str::from_utf8(s).unwrap().to_string());
        true
    }

    fn on_header_value(&mut self, _parser: &mut Parser, s: &[u8]) -> bool {
        self.headers.insert(self.current_key.clone().unwrap(), std::str::from_utf8(s).unwrap().to_string());
        true
    }
}

impl HttpParser {
    fn new() -> HttpParser {
        HttpParser {
            current_key: None,
            headers: HashMap::new()
        }
    }
}

#[derive(PartialEq)]
enum ClientState {
    AwaitingHandshake(HttpParser),
    HandshakeResponse,
    Connected
}

struct WebSocketClient {
    socket: TcpStream,
    state: ClientState,
    ready: Ready,
    headers: HashMap<String, String>,
    outgoing: Vec<WebSocketFrame>
}

impl WebSocketClient {
    pub fn new(socket: TcpStream) -> WebSocketClient {
        WebSocketClient {
            socket: socket,
            state: ClientState::AwaitingHandshake(HttpParser::new()),
            ready: Ready::readable() | UnixReady::hup(),
            headers: HashMap::new(),
            outgoing: Vec::new()
        }
    }

    pub fn write(&mut self) {
        match self.state {
            ClientState::HandshakeResponse => {
                self.write_handshake();
            },
            ClientState::Connected => {
                println!("sending {} frames", self.outgoing.len());

                for frame in self.outgoing.iter() {
                    if let Err(e) = frame.write(&mut self.socket) {
                        println!("error on write: {}", e);
                    }
                }

                self.outgoing.clear();

                self.ready.remove(Ready::writable());
                self.ready.insert(Ready::readable());
            },
            _ => {}
        }
    }

    pub fn write_handshake(&mut self) {
        let response_key = gen_key(&self.headers.get("Sec-WebSocket-Key").unwrap());
        let response = fmt::format(format_args!("HTTP/1.1 101 Switching Protocols\r\n\
                                                Connection: Upgrade\r\n\
                                                Sec-WebSocket-Accept: {}\r\n\
                                                Upgrade: websocket\r\n\r\n", response_key));
        self.socket.write(response.as_bytes()).unwrap();

        self.ready.remove(Ready::writable());
        self.ready.insert(Ready::readable());

        self.state = ClientState::Connected;
    }

    pub fn read(&mut self) {
        match self.state {
            ClientState::AwaitingHandshake(_) => {
                self.read_handshake();
            },
            ClientState::Connected => {
                loop {
                    let frame = WebSocketFrame::read(&mut self.socket);
                    match frame {
                        Ok(frame) => {
                            println!("{:?}", frame);

                            // Add a reply frame to the queue:
                            let reply_frame = WebSocketFrame::from("Hi there!");
                            self.outgoing.push(reply_frame);

                            // Switch the event subscription to the write mode if the queue is not empty:
                            if self.outgoing.len() > 0 {
                                self.ready.remove(Ready::readable());
                                self.ready.insert(Ready::writable());
                            }
                        }
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
                        }
                    }
                }
            },
            _ => {
                // Nothing
            }
        }
    }

    pub fn read_handshake(&mut self) {
        let mut parser = Parser::request();
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
                },
                Ok(len) => {
                    println!("Bytes read: {}\nRead {:?}", len, std::str::from_utf8(&buf[0..len]));
                    println!("Buffer bytes: {:?}", &buf[0..len]);
                    if let ClientState::AwaitingHandshake(ref mut http_parser) = self.state {
                        parser.parse(http_parser, &buf[0..len]);
                    }

                    if parser.is_upgrade() {
                        println!("Is HTTP upgrade");
                        if let ClientState::AwaitingHandshake(ref http_parser) = self.state {
                            self.headers = http_parser.headers.clone();
                        }

                        self.ready.remove(Ready::readable());
                        self.ready.insert(Ready::writable());

                        self.state = ClientState::HandshakeResponse;
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
    }
}

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

        poll.register(&client, new_token, client.ready, PollOpt::edge() | PollOpt::oneshot()).unwrap();
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
            let token = event.token();

            let readiness = event.readiness();
            if UnixReady::from(readiness).is_hup() {
                websocket.clients.remove(&token); // Shutdown using TcpListener method
                println!("Removing closed client from clients");
                continue;
            }

            if readiness.is_readable() {
                match token {
                    SERVER => {
                        println!("Received event from websocket");
                        let _ = websocket.accept(&poll);
                    }
                    client_token => {
                        println!("Received event from {:?}", client_token);
                        let mut client = websocket.clients.get_mut(&client_token).unwrap();
                        client.read();
                        client.reregister(&poll, client_token, client.ready, PollOpt::edge() | PollOpt::oneshot()).unwrap();
                    }
                }
            }

            if readiness.is_writable() {
                let mut client = websocket.clients.get_mut(&token).unwrap();
                client.write();
                client.reregister(&poll, token, client.ready, PollOpt::edge() | PollOpt::oneshot()).unwrap()
            }
        }
    }
}
