use std::env;
use std::str::FromStr;

use capycoin::message::ServerMessage;
use uuid::Uuid;

use capycoin::account::seed_account_id;
use capycoin::{client_util, message::ClientMessage};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Expected two arguments: RECEIVER_ACCOUNT_ID AMOUNT");
        return;
    }

    let rx_account = Uuid::from_str(&args[1].as_str()).unwrap();
    let amount: i64 = args[2].parse().unwrap();

    if amount > 100 {
        eprintln!("Requested amount over the limit!");
        return;
    }

    let seed_account_id = seed_account_id();
    let msg = ClientMessage::CreateTransaction {
        sender_id: seed_account_id,
        receiver_id: rx_account,
        amount: amount,
    };

    let resp = client_util::send_and_recv_msg(msg).expect("Expected a response from core!");

    match resp {
        ServerMessage::CreateTransactionResponse(Ok(r)) => {
            println!("Transaction receipt: {:?}", r)
        }
        ServerMessage::CreateTransactionResponse(Err(err)) => {
            eprintln!("Transaction error: {:?}", err)
        }
        _ => panic!("Unexpected response from server!"),
    }
}
