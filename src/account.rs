use std::fmt;
use std::str::FromStr;

use rusqlite::{params, Connection};

use serde::Deserialize;
use serde::Serialize;

use time::macros::*;
use time::serde::rfc3339;
use time::OffsetDateTime;

use uuid::Uuid;

#[derive(Debug, PartialEq)]
pub enum AccountError {
    SQLError(rusqlite::Error),
    AccountAlreadyExists,
    AccountDoesNotExist,
}

impl std::error::Error for AccountError {}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AccountError: {:?}", self)
    }
}

impl From<rusqlite::Error> for AccountError {
    fn from(error: rusqlite::Error) -> Self {
        AccountError::SQLError(error)
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub account_id: Uuid,
    #[serde(with = "rfc3339")]
    pub created: OffsetDateTime,
    pub funds: i64,
}

impl Account {
    pub fn new() -> Account {
        let account_id = Uuid::new_v4();
        Account {
            account_id: account_id,
            created: time::OffsetDateTime::now_utc(),
            funds: 0,
        }
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountCreatedResult {
    pub account_id: Uuid,
    #[serde(with = "rfc3339")]
    pub timestamp: OffsetDateTime,
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Account(id: {} funds: {} created: {})",
            self.account_id, self.funds, self.created
        )
    }
}

impl fmt::Display for AccountCreatedResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AccountCreatedResult(id: {} timestamp: {})", self.account_id, self.timestamp)
    }
}

pub fn create_accounts_table(con: &mut Connection) -> Result<(), AccountError> {
    let tx = con.transaction()?;

    tx.execute(
        "
        CREATE TABLE IF NOT EXISTS accounts (
            account_id          Uuid             PRIMARY KEY    CHECK(length(account_id) = 16),
            created             OffsetDateTime   NOT NULL,
            funds               i64              NOT NULL       CHECK (funds >= 0)
        )",
        [],
    )?;

    tx.execute("DROP VIEW IF EXISTS v_accounts", [])?;

    tx.execute(
        "CREATE VIEW v_accounts (
            account_id,
            funds,
            created
        ) AS SELECT
            lower(hex(account_id)),
            funds,
            created
        FROM accounts",
        [],
    )?;

    tx.commit()?;

    Ok(())
}

pub fn seed_account_id() -> Uuid {
    Uuid::from_str("4e9b616a-f11e-48b6-8c2f-9534d482e48e").unwrap()
}

pub fn create_seed_account() -> Account {
    let seed_account_stamp = datetime!(2000-01-01 0:00 UTC);
    let seed_account_id = seed_account_id();
    Account {
        account_id: seed_account_id,
        created: seed_account_stamp,
        funds: 1000 * 100,
    }
}

pub fn save_account(con: &mut Connection, account: &Account) -> Result<AccountCreatedResult, AccountError> {
    let tx = con.transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)?;

    match tx.execute(
        "INSERT INTO accounts (account_id, created, funds) VALUES (?, ?, ?)",
        params![account.account_id, account.created, account.funds],
    ) {
        Ok(_) => (),
        Err(rusqlite::Error::SqliteFailure(rc, message)) => match rc.code {
            rusqlite::ErrorCode::ConstraintViolation => return Err(AccountError::AccountAlreadyExists),
            _ => return Err(rusqlite::Error::SqliteFailure(rc, message).into()),
        },
        Err(err) => return Err(err.into()),
    };

    tx.commit()?;

    Ok(AccountCreatedResult {
        account_id: account.account_id,
        timestamp: time::OffsetDateTime::now_utc(),
    })
}

pub fn load_account(con: &mut Connection, account_id: &Uuid) -> Result<Account, AccountError> {
    let account = match con.query_row(
        "SELECT account_id, created, funds FROM accounts WHERE account_id = (?)",
        params![account_id],
        |r| {
            Ok(Account {
                account_id: r.get(0)?,
                created: r.get(1)?,
                funds: r.get(2)?,
            })
        },
    ) {
        Ok(acct) => acct,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Err(AccountError::AccountDoesNotExist),
        Err(err) => return Err(err.into()),
    };

    Ok(account)
}

pub fn get_accounts(con: &mut Connection) -> Result<Vec<Account>, AccountError> {
    let mut tx: Vec<Account> = Vec::new();

    let mut stmt = con.prepare("SELECT account_id, created, funds FROM accounts")?;
    let mut rows = stmt.query(params![])?;

    while let Some(row) = rows.next()? {
        tx.push(Account {
            account_id: row.get(0)?,
            created: row.get(1)?,
            funds: row.get(2)?,
        });
    }

    Ok(tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_account() {
        let a1 = Account::new();
        let a2 = Account::new();

        assert_ne!(a1.account_id, a2.account_id, "account ids should be unique");
        assert_ne!(a1.created, a2.created);
        assert_eq!(a1.funds, 0);
    }

    #[test]
    fn test_create_accounts_table() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();

        create_accounts_table(&mut con).unwrap();

        let n: i64 = con
            .query_row(
                "select count(*) from sqlite_master where type='table' and name='accounts'",
                params![],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn test_create_seed_account() {
        let s = create_seed_account();
        assert_eq!(s.account_id, seed_account_id());
        assert_eq!(s.created, datetime!(2000-01-01 0:00 UTC));
        assert_eq!(s.funds, 100000);
    }

    #[test]
    fn test_save_account() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let a = Account::new();
        let created = save_account(&mut con, &a).unwrap();

        assert_eq!(created.account_id, a.account_id);
        assert!(created.timestamp > a.created);

        let n: i64 = con
            .query_row("select count(*) from accounts where account_id=(?)", params![a.account_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn test_save_account_account_already_exists() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let a = Account::new();
        save_account(&mut con, &a).unwrap();
        let err = save_account(&mut con, &a).expect_err("should not allow duplicate account ids");
        assert_eq!(err, AccountError::AccountAlreadyExists);
    }

    #[test]
    fn test_load_account() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let a = Account::new();
        save_account(&mut con, &a).unwrap();

        let b: Account = load_account(&mut con, &a.account_id).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_load_account_does_not_exist() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let account_id = Uuid::new_v4();
        let err = load_account(&mut con, &account_id).expect_err("should not allow loading an unknown account");
        assert_eq!(err, AccountError::AccountDoesNotExist);
    }

    #[test]
    fn test_get_accounts_none() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let accts = get_accounts(&mut con).unwrap();
        assert_eq!(accts.len(), 0);
    }

    #[test]
    fn test_get_accounts() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_accounts_table(&mut con).unwrap();

        let a1 = Account::new();
        save_account(&mut con, &a1).unwrap();

        let a2 = Account::new();
        save_account(&mut con, &a2).unwrap();

        let accts = get_accounts(&mut con).unwrap();
        assert_eq!(accts.len(), 2);

        assert_eq!(a1, accts[0]);
        assert_eq!(a2, accts[1]);
    }
}
