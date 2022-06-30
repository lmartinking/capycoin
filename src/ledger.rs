use std::fmt;

use serde::{Deserialize, Serialize};
use time::serde::rfc3339;
use time::OffsetDateTime;
use uuid::Uuid;

use rusqlite::{params, Connection};

#[derive(Debug, PartialEq)]
pub enum TransactionError {
    SQLError(rusqlite::Error),
    AmountIsZero,
    AmountIsNegative,
    FeeIsNegative,
    SenderAccountNotEnoughFunds,
    SenderAccountDoesNotExist,
    ReceiverAccountDoesNotExist,
    SenderReceiverAreTheSame,
    TransactionDoesNotExist,
    PermissionDenied,
}

impl std::error::Error for TransactionError {}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "TransactionError: {:?}", self)
    }
}

impl From<rusqlite::Error> for TransactionError {
    fn from(error: rusqlite::Error) -> Self {
        TransactionError::SQLError(error)
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub transaction_id: Uuid,
    #[serde(with = "rfc3339")]
    pub timestamp: OffsetDateTime,
    pub sender_account_id: Uuid,
    pub receiver_account_id: Uuid,
    pub amount: i64,
    pub fee: i32,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub transaction_id: Uuid,
    #[serde(with = "rfc3339")]
    pub timestamp: OffsetDateTime,
    pub sender_account_id: Uuid,
    pub receiver_account_id: Uuid,
    pub sender_account_funds: i64,
    pub fee: i32,
}

impl Transaction {
    pub fn new(sender: Uuid, receiver: Uuid, amount: i64, fee: i32) -> Transaction {
        Transaction {
            transaction_id: Uuid::new_v4(),
            timestamp: time::OffsetDateTime::now_utc(),
            sender_account_id: sender,
            receiver_account_id: receiver,
            amount: amount,
            fee: fee,
        }
    }
}

impl fmt::Display for Transaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Transaction(id: {} timestamp: {} sender: {} receiver: {} amount: {} fee: {})",
            self.transaction_id, self.timestamp, self.sender_account_id, self.receiver_account_id, self.amount, self.fee
        )
    }
}

pub fn create_ledger_table(con: &mut Connection) -> Result<(), TransactionError> {
    let tx = con.transaction()?;

    tx.execute(
        "
        CREATE TABLE IF NOT EXISTS transactions (
            transaction_id      Uuid             PRIMARY KEY    CHECK(length(transaction_id) = 16),
            timestamp           OffsetDateTime   NOT NULL,
            sender_account_id   Uuid             NOT NULL       CHECK(length(sender_account_id) = 16),
            receiver_account_id Uuid             NOT NULL       CHECK(length(receiver_account_id) = 16),
            amount              i64              NOT NULL       CHECK (amount > 0),
            fee                 i32              NOT NULL       CHECK (fee >= 0)
        )
    ",
        [],
    )?;

    tx.execute(
        "CREATE INDEX IF NOT EXISTS transactions_sender_account_id_idx ON transactions (sender_account_id)",
        [],
    )?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS transactions_receiver_account_id_idx ON transactions (receiver_account_id)",
        [],
    )?;

    tx.execute("DROP VIEW IF EXISTS v_transactions", [])?;

    tx.execute(
        "CREATE VIEW v_transactions (
            transaction_id,
            timestamp,
            sender_account_id,
            receiver_account_id,
            amount,
            fee
        ) AS
        SELECT
            lower(hex(transaction_id)),
            timestamp,
            lower(hex(sender_account_id)),
            lower(hex(receiver_account_id)),
            amount,
            fee
        FROM transactions;",
        [],
    )?;

    tx.commit()?;

    Ok(())
}

pub fn save_transaction(con: &mut Connection, transaction: &Transaction) -> Result<TransactionReceipt, TransactionError> {
    // Basic sanity checks
    if transaction.amount == 0 {
        return Err(TransactionError::AmountIsZero);
    }

    if transaction.amount < 0 {
        return Err(TransactionError::AmountIsNegative);
    }

    if transaction.fee < 0 {
        return Err(TransactionError::FeeIsNegative);
    }

    if transaction.sender_account_id == transaction.receiver_account_id {
        return Err(TransactionError::SenderReceiverAreTheSame);
    }

    // TODO: fees!
    assert!(transaction.fee == 0, "Not yet implemented!");

    let tx = con.transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)?;

    // Check sender funds
    let tx_funds: i64 = match tx.query_row(
        "SELECT funds FROM accounts WHERE account_id = (?)",
        params![&transaction.sender_account_id],
        |r| r.get(0),
    ) {
        Ok(r) => r,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(TransactionError::SenderAccountDoesNotExist),
        Err(err) => return Err(err.into()),
    };

    let tx_funds = {
        let deducted = transaction.amount + transaction.fee as i64;
        tx_funds - deducted
    };

    if tx_funds < 0 {
        return Err(TransactionError::SenderAccountNotEnoughFunds);
    }

    // Sender account
    match tx.execute(
        "UPDATE accounts SET funds = (?) WHERE account_id = (?)",
        params![tx_funds, transaction.sender_account_id],
    ) {
        Ok(0) => return Err(TransactionError::SenderAccountDoesNotExist),
        Ok(_) => (),
        Err(err) => return Err(err.into()),
    }

    // Receiver account
    match tx.execute(
        "UPDATE accounts SET funds = funds + (?) WHERE account_id = (?)",
        params![transaction.amount, transaction.receiver_account_id],
    ) {
        Ok(0) => return Err(TransactionError::ReceiverAccountDoesNotExist),
        Ok(_) => (),
        Err(err) => return Err(err.into()),
    }

    let receipt_timestamp = time::OffsetDateTime::now_utc();

    // Update ledger
    tx.execute(
        "INSERT INTO transactions VALUES (?, ?, ?, ?, ?, ?)",
        params![
            transaction.transaction_id,
            // Should we store the transaction.timestamp as well as `requested_timestamp` ?
            receipt_timestamp, // Do not use transaction.timestamp as we are the authority here
            transaction.sender_account_id,
            transaction.receiver_account_id,
            transaction.amount,
            transaction.fee
        ],
    )?;

    tx.commit()?;

    Ok(TransactionReceipt {
        transaction_id: transaction.transaction_id,
        timestamp: receipt_timestamp,
        sender_account_id: transaction.sender_account_id,
        receiver_account_id: transaction.receiver_account_id,
        sender_account_funds: tx_funds,
        fee: transaction.fee,
    })
}

pub fn get_transactions(
    con: &mut Connection,
    account_id: &Uuid,
    time_range: (OffsetDateTime, OffsetDateTime),
) -> Result<Vec<Transaction>, TransactionError> {
    let mut tx: Vec<Transaction> = Vec::new();

    let (begin_time, end_time) = time_range;

    let mut stmt = con.prepare(
        "SELECT transaction_id, timestamp, sender_account_id, receiver_account_id, amount, fee
        FROM transactions
        WHERE (sender_account_id = (?) or receiver_account_id = (?))
        AND timestamp >= (?) AND timestamp <= (?)",
    )?;
    let mut rows = stmt.query(params![account_id, account_id, begin_time, end_time])?;

    while let Some(row) = rows.next()? {
        tx.push(Transaction {
            transaction_id: row.get(0)?,
            timestamp: row.get(1)?,
            sender_account_id: row.get(2)?,
            receiver_account_id: row.get(3)?,
            amount: row.get(4)?,
            fee: row.get(5)?,
        });
    }

    Ok(tx)
}

pub fn get_transaction(con: &mut Connection, account_id: &Uuid, transaction_id: &Uuid) -> Result<Transaction, TransactionError> {
    let tx: Transaction = match con.query_row(
        "SELECT transaction_id, timestamp, sender_account_id, receiver_account_id, amount, fee
        FROM transactions WHERE transaction_id = (?)",
        params![transaction_id],
        |r| {
            Ok(Transaction {
                transaction_id: r.get(0)?,
                timestamp: r.get(1)?,
                sender_account_id: r.get(2)?,
                receiver_account_id: r.get(3)?,
                amount: r.get(4)?,
                fee: r.get(5)?,
            })
        },
    ) {
        Ok(tx) => tx,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(TransactionError::TransactionDoesNotExist),
        Err(err) => return Err(err.into()),
    };

    // Validate that the caller relates to either the sender or receiver of the transaction
    if tx.sender_account_id == *account_id || tx.receiver_account_id == *account_id {
        Ok(tx)
    } else {
        Err(TransactionError::PermissionDenied)
    }
}

#[cfg(test)]
mod tests {
    use crate::account::{create_accounts_table, save_account, Account};

    use super::*;

    #[test]
    fn test_new_transaction() {
        let tx1 = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), 100, 0);
        let tx2 = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), 100, 0);
        assert_ne!(tx1.transaction_id, tx2.transaction_id);
        assert_ne!(tx1.timestamp, tx2.timestamp);
    }

    #[test]
    fn test_create_ledger_table() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();

        create_ledger_table(&mut con).unwrap();

        let n: i64 = con
            .query_row(
                "select count(*) from sqlite_master where type='table' and name='transactions'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn test_transaction_save_get() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();

        create_ledger_table(&mut con).unwrap();
        create_accounts_table(&mut con).unwrap();

        let mut tx_account = Account::new();
        tx_account.funds = 100;

        let rx_account = Account::new();

        save_account(&mut con, &tx_account).unwrap();
        save_account(&mut con, &rx_account).unwrap();

        let tx = Transaction::new(tx_account.account_id, rx_account.account_id, 100, 0);
        save_transaction(&mut con, &tx).unwrap();

        let txs = get_transactions(
            &mut con,
            &tx_account.account_id,
            (OffsetDateTime::UNIX_EPOCH, OffsetDateTime::now_utc()),
        )
        .unwrap();
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn test_transaction_sender_not_exist() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_ledger_table(&mut con).unwrap();
        create_accounts_table(&mut con).unwrap();

        let tx = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), 100, 0);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow this");
        assert_eq!(err, TransactionError::SenderAccountDoesNotExist);
    }

    #[test]
    fn test_transaction_sender_not_enough_funds() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_ledger_table(&mut con).unwrap();
        create_accounts_table(&mut con).unwrap();

        let sender = Account::new();
        save_account(&mut con, &sender).unwrap();

        let tx = Transaction::new(sender.account_id, Uuid::new_v4(), 100, 0);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow this");
        assert_eq!(err, TransactionError::SenderAccountNotEnoughFunds);
    }

    #[test]
    fn test_transaction_receiver_not_exist() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_ledger_table(&mut con).unwrap();
        create_accounts_table(&mut con).unwrap();

        let mut sender = Account::new();
        sender.funds = 100;
        save_account(&mut con, &sender).unwrap();

        let tx = Transaction::new(sender.account_id, Uuid::new_v4(), 100, 0);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow this");
        assert_eq!(err, TransactionError::ReceiverAccountDoesNotExist);
    }

    #[test]
    fn test_transaction_negative_amount() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        let tx = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), -100, 0);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow negative amounts!");
        assert_eq!(err, TransactionError::AmountIsNegative);
    }

    #[test]
    fn test_transaction_zero_amount() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        let tx = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), 0, 0);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow zero amounts!");
        assert_eq!(err, TransactionError::AmountIsZero);
    }

    #[test]
    fn test_transaction_negative_fee() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        let tx = Transaction::new(Uuid::new_v4(), Uuid::new_v4(), 100, -10);
        let err = save_transaction(&mut con, &tx).expect_err("should not allow negative fees!");
        assert_eq!(err, TransactionError::FeeIsNegative);
    }

    #[test]
    fn test_get_transactions_none() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_ledger_table(&mut con).unwrap();

        let txs = get_transactions(&mut con, &Uuid::new_v4(), (OffsetDateTime::UNIX_EPOCH, OffsetDateTime::now_utc())).unwrap();
        assert_eq!(txs.len(), 0);
    }

    #[test]
    fn test_get_transactions_none_in_range() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();

        create_ledger_table(&mut con).unwrap();
        create_accounts_table(&mut con).unwrap();

        let mut tx_account = Account::new();
        tx_account.funds = 100;

        let rx_account = Account::new();

        save_account(&mut con, &tx_account).unwrap();
        save_account(&mut con, &rx_account).unwrap();

        let tx = Transaction::new(tx_account.account_id, rx_account.account_id, 100, 0);
        save_transaction(&mut con, &tx).unwrap();

        let txs = get_transactions(
            &mut con,
            &tx_account.account_id,
            (OffsetDateTime::now_utc(), OffsetDateTime::now_utc()),
        )
        .unwrap();
        assert_eq!(txs.len(), 0);
    }
}
