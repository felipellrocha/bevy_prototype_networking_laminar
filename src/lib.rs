use bevy::prelude::*;

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use laminar::{Config, Socket};

use bytes::Bytes;
use uuid::Uuid;

mod worker;

pub struct NetworkingPlugin;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct SocketHandle(uuid::Uuid);

impl SocketHandle {
    fn new() -> Self {
        // We're using UUID here to mirror the way bevy currently treats asset handles. Since sockets handles are specific to a single process, and it's
        // unlikely anyone will have a large number of sockets, we could switching to a u32.
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct Connection {
    pub addr: SocketAddr,
    pub socket: SocketHandle,
}

#[derive(Debug)]
pub enum NetworkEvent {
    Connected(Connection),
    Disconnected(Connection),
    Message(Connection, Bytes),
}
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum NetworkDelivery {
    UnreliableUnordered,
    UnreliableSequenced(Option<u8>),
    ReliableUnordered,
    ReliableSequenced(Option<u8>),
    ReliableOrdered(Option<u8>),
}

pub struct NetworkResource {
    default_socket: Option<SocketHandle>,

    bound_sockets: Vec<SocketHandle>,
    connections: Vec<Connection>,
    event_rx: Mutex<Receiver<NetworkEvent>>,
    message_tx: Mutex<Sender<Message>>,
    instruction_tx: Mutex<Sender<SocketInstruction>>,
}

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut AppBuilder) {
        let network_resource = worker::start_worker_thread();

        app.add_event::<NetworkEvent>()
            .add_resource(network_resource)
            .add_system(process_network_events.system());
    }
}

impl NetworkResource {
    pub fn connections(&self) -> &Vec<Connection> {
        return &self.connections;
    }

    pub fn connections_for_socket(&self, socket: SocketHandle) -> Vec<Connection> {
        return self
            .connections
            .iter()
            .filter(|c| c.socket == socket)
            .map(|c| *c)
            .collect();
    }

    pub fn add_connection(&mut self, connection: Connection) {
        if self.has_connection(connection) {
            println!("Warning: attempted to add a connection that already exists");
            return;
        }

        self.connections.push(connection);
    }

    pub fn remove_connection(&mut self, connection: Connection) {
        let conn = self.connections.iter().position(|c| *c == connection);

        match conn {
            Some(idx) => {
                self.connections.remove(idx);
            }
            None => {
                println!("Warning: attempted to remove connection that doesn't exist");
            }
        }
    }

    pub fn has_connection(&self, connection: Connection) -> bool {
        self.connections.iter().any(|c| *c == connection)
    }

    pub fn bind<A: ToSocketAddrs>(&mut self, addr: A) -> Result<SocketHandle, ()> {
        let mut cfg = Config::default();
        cfg.idle_connection_timeout = Duration::from_millis(2000);
        cfg.heartbeat_interval = Some(Duration::from_millis(1000));
        cfg.max_packets_in_flight = 2048;

        let handle = SocketHandle::new();
        let socket = Socket::bind_with_config(addr, cfg).expect("socket bind failed"); // todo

        let instruction = SocketInstruction::AddSocket(handle, socket);
        {
            let locked = self.instruction_tx.lock().expect("instruction lock failed");
            locked.send(instruction).expect("instruction send failed");
        }

        if self.default_socket.is_none() {
            self.default_socket = Some(handle);
        }

        return Ok(handle);
    }

    pub fn send(&self, addr: SocketAddr, message: &[u8], delivery: NetworkDelivery) {
        self.send_with_config(addr, message, delivery, SendConfig::default());
    }

    pub fn broadcast(&self, message: &[u8], delivery: NetworkDelivery) {
        self.broadcast_with_config(message, delivery, SendConfig::default());
    }

    pub fn send_with_config(
        &self,
        addr: SocketAddr,
        message: &[u8],
        delivery: NetworkDelivery,
        config: SendConfig,
    ) {
        let socket = self.get_socket_or_default(config.socket);

        let msg = Message {
            destination: addr,
            delivery: delivery,
            socket_handle: socket,
            message: Bytes::copy_from_slice(message),
        };

        self.message_tx.lock().unwrap().send(msg).unwrap();
    }

    pub fn broadcast_with_config(
        &self,
        message: &[u8],
        delivery: NetworkDelivery,
        config: SendConfig,
    ) {
        let socket = self.get_socket_or_default(config.socket);

        let broadcast_to = self.connections_for_socket(socket);

        for conn in broadcast_to {
            let msg = Message {
                destination: conn.addr,
                delivery: delivery,
                socket_handle: socket,
                message: Bytes::copy_from_slice(message),
            };

            self.message_tx.lock().unwrap().send(msg).unwrap();
        }
    }

    fn get_socket_or_default(&self, socket: Option<SocketHandle>) -> SocketHandle {
        let socket = socket
            .or(self.default_socket)
            .expect("No socket handle was provided and no default socket is bound");

        if self.bound_sockets.contains(&socket) {
            panic!("Cannot send on the provided socket handle: there is no open socket bound for that handle.");
        }

        // we've turned the socket into the default, and confirmed that it was bound.
        return socket;
    }
}

#[derive(Default)]
pub struct SendConfig {
    pub socket: Option<SocketHandle>, // if none, use the default socket
}

#[derive(Debug)]
struct Message {
    message: Bytes,
    delivery: NetworkDelivery,
    socket_handle: SocketHandle,
    destination: SocketAddr,
}

enum SocketInstruction {
    AddSocket(SocketHandle, Socket),
}

fn process_network_events(
    mut net: ResMut<NetworkResource>,
    mut network_events: ResMut<Events<NetworkEvent>>,
) {
    let mut added_connections: Vec<Connection> = Vec::new();
    let mut removed_connections: Vec<Connection> = Vec::new();

    {
        let locked = net.event_rx.lock().unwrap();

        while let Ok(event) = locked.try_recv() {
            match event {
                NetworkEvent::Connected(conn) => {
                    if !net.has_connection(conn) && !added_connections.contains(&conn) {
                        added_connections.push(conn);
                    }
                }
                NetworkEvent::Disconnected(conn) => {
                    if net.has_connection(conn) && !removed_connections.contains(&conn) {
                        removed_connections.push(conn);
                    }
                }
                _ => network_events.send(event),
            }
        }
    }

    for conn in added_connections {
        net.add_connection(conn);
        network_events.send(NetworkEvent::Connected(conn));
    }

    for conn in removed_connections {
        net.remove_connection(conn);
        network_events.send(NetworkEvent::Disconnected(conn));
    }
}
