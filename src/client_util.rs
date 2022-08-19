use std::fs;
use std::os::unix::net::UnixDatagram;

use std::time::{Duration, Instant};

use nix::sys::socket::setsockopt;
use nix::sys::socket::sockopt::RcvBuf;
use std::os::unix::io::AsRawFd;

use std::process;

use crate::common;
use crate::message::{ClientMessage, ServerMessage, ServerMessagePacket};

lazy_static! {
    static ref START_TIME: Instant = Instant::now();
}

// Create a socket address for client usage
fn make_client_sock_address() -> String {
    let pid = process::id();
    let ts = Instant::now().duration_since(*START_TIME).as_millis();
    let sock_path = format!("/tmp/capycoin-client.{}.{}", pid, ts);
    println!("Sock path: {}", sock_path);
    sock_path
}

// Send a `ClientMessage` and try and receive a `ServerMessage`.
pub fn send_and_recv_msg(message: ClientMessage) -> Option<ServerMessage> {
    let client_sock_address = make_client_sock_address();
    let server_sock_address = common::CORE_SOCKET_PATH;

    let sock = match UnixDatagram::bind(&client_sock_address) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error creating socket: {}", err);
            return None;
        }
    };

    sock.set_read_timeout(Some(Duration::new(1, 0)))
        .expect("Could not set socket timeout!");

    let fd = sock.as_raw_fd();
    setsockopt(fd, RcvBuf, &(4 * 1024 * 1024)).expect("setsockopt - rcvbuf!");

    match sock.connect(server_sock_address) {
        Ok(sock) => sock,
        Err(err) => {
            eprintln!("Could not connect: {:?}", err);
            return None;
        }
    }

    println!("Connected to: {}", server_sock_address);

    let message_packet = message.to_packet();
    let message_ser = message_packet.to_json().expect("did not serialise!");

    println!("ClientMessage: `{}`", message_ser);

    // Request
    sock.send(message_ser.as_bytes()).unwrap();

    // Response
    let mut buf: Vec<u8> = Vec::with_capacity(1024 * 1024);
    buf.resize(buf.capacity(), 0);

    let recv_size = match sock.recv(&mut buf) {
        Ok(sz) => sz,
        Err(err) => {
            eprintln!("Error receiving server response! {}", err);
            return None;
        }
    };

    let msg = &buf[..recv_size];
    println!("Received {} bytes", recv_size);

    let resp = match std::str::from_utf8(msg) {
        Err(err) => {
            eprintln!("Error decoding data from server: {}", err);
            None
        }
        Ok(r) => ServerMessagePacket::from_json(r),
    };

    // Disconnect
    _ = sock
        .shutdown(std::net::Shutdown::Both)
        .map_err(|e| eprintln!("Error shutting down socket: {}", e));

    // Remove socket
    _ = fs::remove_file(client_sock_address.as_str()).map_err(|e| eprintln!("Error removing socket: {}", e));

    let resp = match resp {
        Some(r) => r,
        None => return None,
    };

    if resp.message_id != message_packet.message_id {
        eprintln!("Mismatched message ID from server response!");
        return None;
    }

    Some(resp.message)
}
