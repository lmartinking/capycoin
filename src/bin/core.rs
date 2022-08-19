use std::fs;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::os::unix::prelude::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

use nix::sys::socket::setsockopt;
use nix::sys::socket::sockopt::{RcvBuf, SndBuf};

use rusqlite::Connection;

use capycoin::account::{self, Account};
use capycoin::common;
use capycoin::ledger::{self, Transaction};

use capycoin::message::ClientMessage;
use capycoin::message::{ClientMessagePacket, ServerMessage, ServerMessagePacket};

#[derive(Debug)]
enum AppError {
    LedgerError(ledger::TransactionError),
    AccountError(account::AccountError),
    SQLError(rusqlite::Error),
    SocketError(std::io::Error),
    GeneralError,
}

impl std::error::Error for AppError {}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AppError: {:?}", self)
    }
}

impl From<ledger::TransactionError> for AppError {
    fn from(error: ledger::TransactionError) -> Self {
        AppError::LedgerError(error)
    }
}

impl From<account::AccountError> for AppError {
    fn from(error: account::AccountError) -> Self {
        AppError::AccountError(error)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(error: rusqlite::Error) -> Self {
        AppError::SQLError(error)
    }
}

impl From<std::str::Utf8Error> for AppError {
    fn from(error: std::str::Utf8Error) -> Self {
        eprint!("GeneralError from: {:?}", error);
        AppError::GeneralError
    }
}

impl From<std::io::Error> for AppError {
    fn from(error: std::io::Error) -> Self {
        AppError::SocketError(error)
    }
}

fn socket_server<F>(socket_path: &str, timeout: Option<Duration>, mut func: F) -> Result<(), AppError>
where
    F: FnMut(SocketAddr, &[u8]) -> Result<Option<Vec<u8>>, AppError>,
{
    if Path::new(socket_path).exists() {
        eprintln!("Socket already exists, cleaning up...");
        fs::remove_file(socket_path)?;
    }

    println!("Setting up socket.");

    let socket = UnixDatagram::bind(socket_path)?;
    socket.set_read_timeout(timeout)?;

    let fd = socket.as_raw_fd();
    setsockopt(fd, SndBuf, &(4 * 1024 * 1024)).expect("setsockopt - sndbuf!");
    setsockopt(fd, RcvBuf, &(1024 * 1024)).expect("setsockopt - rcvbuf!");

    println!("Listening for messages on: {}", socket_path);

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        eprintln!("Received interrupt.");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let mut buf: Vec<u8> = Vec::with_capacity(1024 * 1024);
    buf.resize(buf.capacity(), 0);

    while running.load(Ordering::SeqCst) {
        let (count, address) = match socket.recv_from(&mut buf) {
            Ok(result) => result,
            Err(err) => {
                match err.kind() {
                    io::ErrorKind::WouldBlock => continue,
                    io::ErrorKind::Interrupted => continue,
                    io::ErrorKind::TimedOut => continue,
                    _ => (),
                };
                eprintln!("Error from `socket.recv_from`: {:?}", err);
                break;
            }
        };

        let msg = &buf[..count];
        println!("socket {:?} received {:?} bytes", address, msg.len());

        let result = func(address.to_owned(), msg);
        println!("handler finished...");

        if let Ok(Some(resp)) = result {
            let client_addr = address.as_pathname();
            if let Some(addr) = client_addr {
                println!("Sending response back to client... (size: {} bytes)", resp.len());

                match socket.send_to(resp.as_slice(), &addr) {
                    Ok(_) => (),
                    Err(err) => {
                        eprintln!("Error sending response to clinet: {:?}", err)
                    }
                };
            }
        } else {
            if result.is_err() {
                println!("Error: {:?}", result);
            }
            println!("No response back to client...");
        }
    }

    socket.shutdown(std::net::Shutdown::Both)?;

    Ok(())
}

fn message_dispatch(con: &mut rusqlite::Connection, message: &ClientMessagePacket) -> ServerMessagePacket {
    let resp: ServerMessage = match message.message {
        ClientMessage::CreateNewAccount => {
            let acct = Account::new();
            let result = account::save_account(con, &acct);
            result.into()
        }
        ClientMessage::GetAccount { account_id } => {
            let result = account::load_account(con, &account_id);
            result.into()
        }
        ClientMessage::GetAccounts => {
            let result = account::get_accounts(con);
            result.into()
        }
        ClientMessage::GetTransactions {
            account_id,
            time_range_start,
            time_range_end,
        } => {
            let result = ledger::get_transactions(con, &account_id, (time_range_start, time_range_end));
            result.into()
        }
        ClientMessage::GetTransaction {
            account_id,
            transaction_id,
        } => {
            let result = ledger::get_transaction(con, &account_id, &transaction_id);
            result.into()
        }
        ClientMessage::CreateTransaction {
            sender_id,
            receiver_id,
            amount,
        } => {
            let tx = Transaction::new(sender_id, receiver_id, amount, 0);
            let result = ledger::save_transaction(con, &tx);
            result.into()
        }
    };

    resp.as_resp_for(message)
}

fn main() -> Result<(), AppError> {
    println!("Starting CapyCoin Core...");

    let mut con = Connection::open("capycoin.db3")?;

    con.pragma_update(None, "journal_mode", "WAL")?;

    println!("Creating accounts table!");
    account::create_accounts_table(&mut con)?;

    println!("Creating ledger table!");
    ledger::create_ledger_table(&mut con)?;

    match account::save_account(&mut con, &account::create_seed_account()) {
        Ok(_) => (),
        Err(err) => eprintln!("Could not create seed account! {}", err),
    }

    socket_server(common::CORE_SOCKET_PATH, Some(Duration::new(1, 0)), |address, msg| {
        println!("handler func received from: {:?} bytes: {:?}", address, msg.len());

        let command = std::str::from_utf8(msg)?;

        println!("Received command: `{}`", command);

        let message = match ClientMessagePacket::from_json(command) {
            Some(msg) => msg,
            None => return Ok(None),
        };

        println!("Received message: `{:?}`", message);

        let resp = message_dispatch(&mut con, &message);
        let resp_json = resp.to_json().expect("error serialising response");

        println!("Response to client: `{}`", resp_json);

        Ok(Some(resp_json.as_bytes().to_vec()))
    })?;

    println!("Shutting down...");

    con.close().unwrap();

    Ok(())
}
