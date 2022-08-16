//
// Client Gateway - Provides a simple REST API bridge to core and handles auth via tokens
//

#[macro_use]
extern crate rouille;
#[macro_use]
extern crate lazy_static;

use std::fs;
use std::os::unix::net::UnixDatagram;

use std::process;
use std::time::{Duration, Instant};

use nix::sys::socket::setsockopt;
use nix::sys::socket::sockopt::RcvBuf;
use std::os::unix::prelude::AsRawFd;

use time::serde::rfc3339;
use time::{format_description, Date, OffsetDateTime};

use rouille::{Request, Response};
use rusqlite::Connection;

use serde::{Deserialize, Serialize};

use uuid::Uuid;

use capycoin::auth;
use capycoin::common;
use capycoin::message::{ClientMessage, ServerError, ServerMessage, ServerMessagePacket};

lazy_static! {
    static ref START_TIME: Instant = Instant::now();
}

fn gateway_error(message: &str) -> ServerError {
    ServerError {
        error_type: "GatewayError".to_string(),
        error_message: message.to_string(),
    }
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
fn send_and_recv_msg(message: ClientMessage) -> Option<ServerMessage> {
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

// Try and extract a token from a HTTP Client request (Bearer Auth)
fn bearer_token(request: &Request) -> Option<&str> {
    let value = request.header("Authorization");
    value?;

    let value = value.unwrap();
    if !value.starts_with("Bearer ") {
        return None;
    }

    let parts = value.split("Bearer ").collect::<Vec<&str>>();
    if parts.len() != 2 {
        return None;
    }
    let token = parts[1];
    Some(token)
}

fn root_handler(_request: &Request) -> Response {
    Response::html("<h1>CapyCoin Gateway!</h1><p><a href='https://github.com/lmartinking/capycoin/blob/master/README.md'>Documentation</a></p>")
}

fn create_account_handler(request: &Request) -> Response {
    println!("Create account handler!");

    // Tokens are not necessary and should not be used because this is a NEW account
    if bearer_token(request).is_some() {
        return Response::json(&gateway_error("Bearer Auth not necessary")).with_status_code(400);
    }

    // TODO: implement signed transactions -- client should provide pub key at account creation
    let res = match send_and_recv_msg(ClientMessage::CreateNewAccount) {
        Some(ServerMessage::CreateNewAccountResponse(Ok(r))) => r,
        Some(ServerMessage::CreateNewAccountResponse(Err(err))) => return Response::json(&err).with_status_code(500),
        _ => return Response::json(&gateway_error("Unexpected response from server")).with_status_code(500),
    };

    let mut con = match Connection::open("auth.db3") {
        Ok(con) => con,
        Err(err) => {
            eprint!("Error opening auth db: {:?}", err);
            return Response::json(&gateway_error("Internal server error")).with_status_code(500);
        }
    };

    let token = match auth::create_token(&mut con, &res.account_id) {
        Ok(t) => t,
        Err(err) => return Response::json(&ServerError::from(err)).with_status_code(500),
    };

    #[derive(PartialEq, Debug, Serialize)]
    struct CreateAccountResponse {
        account_id: Uuid,
        token: String,
        #[serde(with = "rfc3339")]
        token_expiry: OffsetDateTime,
    }

    Response::json(&CreateAccountResponse {
        account_id: res.account_id,
        token: token.token,
        token_expiry: token.expiry,
    })
}

fn validate_token_response(request: &Request, account_id: &Uuid) -> Option<Response> {
    let token_str = match bearer_token(request) {
        None => return Some(Response::json(&gateway_error("Token required!")).with_status_code(401)),
        Some(t) => t,
    };

    let mut con = match Connection::open("auth.db3") {
        Ok(con) => con,
        Err(err) => {
            eprint!("Error opening auth db: {:?}", err);
            return Some(Response::json(&gateway_error("Internal server error")).with_status_code(500));
        }
    };

    match auth::validate_token(&mut con, token_str.to_string(), account_id) {
        Err(err) => {
            eprintln!("Error during token validation: {:?}", err);
            Some(Response::json(&gateway_error("Invalid token!")).with_status_code(401))
        }
        Ok(false) => Some(Response::json(&gateway_error("Invalid token!")).with_status_code(401)),
        Ok(true) => None, // Token valid!
    }
}

fn get_account_handler(request: &Request, account_id: String) -> Response {
    println!("Get account handler!");

    let account_id = match Uuid::parse_str(&account_id) {
        Err(_) => return Response::json(&gateway_error("Invalid format for `account_id`")).with_status_code(400),
        Ok(v) => v,
    };

    if let Some(resp) = validate_token_response(request, &account_id) {
        return resp;
    }

    let account = match send_and_recv_msg(ClientMessage::GetAccount { account_id: account_id }) {
        Some(ServerMessage::GetAccountResponse(Ok(res))) => res,
        _ => return Response::text("Unexpected response from core").with_status_code(500),
    };

    Response::json(&account)
}

// Parse date from a query string value, eg: `YYYY-MM-DD`
fn parse_date_param(param: Option<String>) -> Option<OffsetDateTime> {
    match param {
        Some(val) => {
            let format = format_description::parse("[year]-[month]-[day]").expect("format description");
            let v = Date::parse(&val, &format);
            match v {
                Ok(v) => Some(v.midnight().assume_utc()),
                Err(e) => {
                    eprintln!("parse err: {:?}", e);
                    None
                }
            }
        }
        _ => None,
    }
}

fn get_transactions_handler(request: &Request, account_id: String) -> Response {
    println!("Get transactions handler!");

    let account_id = match Uuid::parse_str(&account_id) {
        Err(_) => return Response::text("Invalid format for `account_id`").with_status_code(400),
        Ok(v) => v,
    };

    if let Some(resp) = validate_token_response(request, &account_id) {
        return resp;
    }

    let start = parse_date_param(request.get_param("start"));
    let end = parse_date_param(request.get_param("end"));

    let (start, end) = match (start, end) {
        (Some(start), Some(end)) => (start, end),
        _ => return Response::text("`start` and `end` params are required!").with_status_code(400),
    };

    let resp = match send_and_recv_msg(ClientMessage::GetTransactions {
        account_id: account_id,
        time_range_start: start,
        time_range_end: end,
    }) {
        Some(ServerMessage::GetTransactionsResponse(Ok(r))) => r,
        Some(ServerMessage::GetTransactionsResponse(Err(r))) => return Response::json(&r).with_status_code(500),
        _ => return Response::json(&gateway_error("Unexpected response from server")).with_status_code(500),
    };

    #[derive(PartialEq, Debug, Serialize)]
    struct GetTransactionsResponse {
        account_id: Uuid,
        transactions: Vec<capycoin::ledger::Transaction>,
    }

    Response::json(&GetTransactionsResponse {
        account_id: account_id,
        transactions: resp,
    })
}

fn create_transaction_handler(request: &Request, account_id: String) -> Response {
    println!("Create transaction: {}", account_id);

    let account_id = match Uuid::parse_str(&account_id) {
        Err(_) => return Response::json(&gateway_error("Invalid format for `account_id`")).with_status_code(400),
        Ok(v) => v,
    };

    if let Some(resp) = validate_token_response(request, &account_id) {
        return resp;
    }

    // TODO: implement signed (ed25519) transactions
    #[derive(Deserialize)]
    struct TransationRequest {
        receiver: Uuid,
        amount: i64,
        // #[serde(with = "rfc3339")]
        // timestamp: OffsetDateTime,
    }

    let tx_request: TransationRequest = match rouille::input::json_input(request) {
        Ok(r) => r,
        Err(_err) => return Response::json(&gateway_error("Unexpected transaction request format")).with_status_code(400),
    };

    let resp = match send_and_recv_msg(ClientMessage::CreateTransaction {
        sender_id: account_id,
        receiver_id: tx_request.receiver,
        amount: tx_request.amount,
    }) {
        Some(ServerMessage::CreateTransactionResponse(Ok(r))) => r,
        Some(ServerMessage::CreateTransactionResponse(Err(r))) => return Response::json(&r).with_status_code(500),
        _ => return Response::json(&gateway_error("Unexpected response from server")).with_status_code(500),
    };

    Response::json(&resp)
}

fn get_transaction_handler(request: &Request, account_id: String, transaction_id: String) -> Response {
    let account_id = match Uuid::parse_str(&account_id) {
        Err(_) => return Response::json(&gateway_error("Invalid format for `account_id`")).with_status_code(400),
        Ok(v) => v,
    };

    let transaction_id = match Uuid::parse_str(&transaction_id) {
        Err(_) => return Response::json(&gateway_error("Invalid format for `transaction_id`")).with_status_code(400),
        Ok(v) => v,
    };

    if let Some(resp) = validate_token_response(request, &account_id) {
        return resp;
    }

    let resp = match send_and_recv_msg(ClientMessage::GetTransaction {
        account_id: account_id,
        transaction_id: transaction_id,
    }) {
        Some(ServerMessage::GetTransactionResponse(Ok(r))) => r,
        Some(ServerMessage::GetTransactionResponse(Err(r))) => return Response::json(&r).with_status_code(500),
        _ => return Response::json(&gateway_error("Unexpected response from server")).with_status_code(500),
    };

    Response::json(&resp)
}

fn main() {
    let hostname = "localhost";
    let port = 8000;
    let listen_str = format!("{}:{}", hostname, port);

    {
        println!("Creating auth table...");
        let mut con = Connection::open("auth.db3").unwrap();
        auth::create_auth_table(&mut con).unwrap();
        con.close().unwrap();
    }

    println!("Now listening on: http://{}", listen_str);

    rouille::start_server(listen_str, move |request| {
        router!(request,
            (GET) (/) => {
                root_handler(request)
            },

            (POST) (/account) => {
                create_account_handler(request)
            },

            (GET) (/account/{account_id: String}) => {
                get_account_handler(request, account_id)
            },

            (GET) (/account/{account_id: String}/transactions) => {
                get_transactions_handler(request, account_id)
            },

            (POST) (/account/{account_id: String}/transaction) => {
                create_transaction_handler(request, account_id)
            },

            (GET) (/account/{account_id: String}/transaction/{transaction_id: String}) => {
                get_transaction_handler(request, account_id, transaction_id)
            },

            _ => Response::empty_404()
        )
    });
}
