use std::env;

use capycoin::message::ServerMessage;

use capycoin::account::{seed_account_id, Account};
use capycoin::{client_util, message::ClientMessage};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Stipiend! Pay out a stipend to accounts at or below a minimum level of funds.");
        eprintln!("Expected an argument: ACCOUNT_MINIMUM STIPEND_AMOUNT");
        return;
    }

    let minimum: i64 = args[1].parse().unwrap();
    let amount: i64 = args[2].parse().unwrap();

    if amount > 100 {
        eprintln!("Requested amount over the limit!");
        return;
    }

    let msg = ClientMessage::GetAccounts;
    let resp = client_util::send_and_recv_msg(msg).expect("Expected a response from core!");

    let accts = match resp {
        ServerMessage::GetAccountsResponse(Ok(r)) => r,
        ServerMessage::GetAccountsResponse(Err(err)) => panic!("{:?}", err),
        _ => panic!("Unexpected response from server!"),
    };

    println!("Found {} accounts", accts.len());

    let accts: Vec<Account> = accts.into_iter().filter(|a| {
        a.funds <= minimum
    }).collect();

    println!("Found {} accounts eligible for stipend!", accts.len());

    let seed_account_id = seed_account_id();

    for acct in &accts {
        println!("Stipend for: {}", acct);

        let msg = ClientMessage::CreateTransaction {
            sender_id: seed_account_id,
            receiver_id: acct.account_id,
            amount: amount,
        };

        let resp = client_util::send_and_recv_msg(msg).expect("Expected a response from core!");

        match resp {
            ServerMessage::CreateTransactionResponse(Ok(r)) => {
                println!("Stipend transaction receipt: {:?}", r)
            }
            ServerMessage::CreateTransactionResponse(Err(err)) => {
                eprintln!("Transaction error: {:?}", err)
            }
            _ => panic!("Unexpected response from server!"),
        }
    }
}
