use bevy::prelude::*;

use bevy_prototype_networking_laminar::{
    NetworkDelivery, NetworkEvent, NetworkResource, NetworkingPlugin, SendConfig, SocketHandle,
};

use std::net::SocketAddr;

const SERVER: &str = "127.0.0.1:12351";
const CLIENT: &str = "127.0.0.1:12350";

fn main() {
    App::build()
        .init_resource::<EventListenerState>()
        .init_resource::<SendTimerState>()
        .init_resource::<Sockets>()
        .add_default_plugins()
        .add_plugin(NetworkingPlugin)
        .add_startup_system(startup_system.system())
        .add_system(print_messages_system.system())
        .add_system(send_messages.system())
        .run();
}

fn print_messages_system(
    mut state: ResMut<EventListenerState>,
    sockets: Res<Sockets>,
    my_events: Res<Events<NetworkEvent>>,
) {
    if let Some(server) = sockets.server {
        if let Some(client) = sockets.client {
            for event in state.network_events.iter(&my_events) {
                match event {
                    NetworkEvent::Message(conn, data) => {
                        let msg = String::from_utf8_lossy(data);

                        let from = if conn.socket == server {
                            "SERVER"
                        } else if conn.socket == client {
                            "CLIENT"
                        } else {
                            "UNKNOWN"
                        };

                        println!("\t ---> [{}] {:?}\n", from, msg);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn startup_system(mut net: ResMut<NetworkResource>, mut sockets: ResMut<Sockets>) {
    sockets.server = Some(net.bind(SERVER).unwrap());
    sockets.client = Some(net.bind(CLIENT).unwrap());
}

fn send_messages(
    time: Res<Time>,
    sockets: Res<Sockets>,
    mut state: ResMut<SendTimerState>,
    net: ResMut<NetworkResource>,
) {
    state.message_timer.tick(time.delta_seconds);
    if state.message_timer.finished {
        let server: SocketAddr = SERVER.parse().unwrap();
        let client: SocketAddr = CLIENT.parse().unwrap();

        let (to, who, message, socket, from_server) = if state.from_server {
            (client, "SERVER", "How are things?", sockets.server, false)
        } else {
            (server, "CLIENT", "Good. Thanks!", sockets.client, true)
        };

        println!("[{}] ---> {:?}", who, message);
        net.send_with_config(
            to,
            message.as_bytes(),
            NetworkDelivery::ReliableSequenced(Some(1)),
            SendConfig { socket: socket },
        );
        state.from_server = from_server;

        state.message_timer.reset();
    }
}

#[derive(Default)]
struct Sockets {
    server: Option<SocketHandle>,
    client: Option<SocketHandle>,
}

#[derive(Default)]
struct EventListenerState {
    network_events: EventReader<NetworkEvent>,
}

struct SendTimerState {
    message_timer: Timer,
    from_server: bool,
}

impl Default for SendTimerState {
    fn default() -> Self {
        SendTimerState {
            message_timer: Timer {
                elapsed: 2.0,
                duration: 2.5,
                repeating: true,
                ..Default::default()
            },
            from_server: true,
        }
    }
}