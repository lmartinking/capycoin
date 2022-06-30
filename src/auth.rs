use rusqlite::{params, Connection};

use core::fmt::Write;
use std::fmt;

use rand;
use uuid::Uuid;

use time::Duration;
use time::OffsetDateTime;

use bcrypt;

#[derive(Debug, PartialEq)]
pub enum AuthError {
    SQLError(rusqlite::Error),
    TokenLengthError,
    TokenParseError(std::num::ParseIntError),
    TokenCryptError,
    TokenExpired,
    TokenDoesNotExist,
}

#[derive(Debug, PartialEq)]
pub struct Token {
    pub bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
pub struct TokenCreatedResult {
    pub token: String,
    pub account_id: Uuid,
    pub expiry: OffsetDateTime,
}

impl std::error::Error for AuthError {}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AuthError: {:?}", self)
    }
}

impl From<rusqlite::Error> for AuthError {
    fn from(error: rusqlite::Error) -> Self {
        AuthError::SQLError(error)
    }
}

impl From<bcrypt::BcryptError> for AuthError {
    fn from(_: bcrypt::BcryptError) -> Self {
        AuthError::TokenCryptError // Discard BcryptError info because it does work with `PartialEq`
    }
}

pub fn create_auth_table(con: &mut Connection) -> Result<(), AuthError> {
    let tx = con.transaction()?;

    tx.execute(
        "
        CREATE TABLE IF NOT EXISTS auth (
            token_hash          TEXT             NOT NULL,
            account_id          Uuid             NOT NULL    CHECK(length(account_id) = 16),
            expiry              OffsetDateTime   NOT NULL
        )",
        [],
    )?;

    tx.execute("DROP VIEW IF EXISTS v_auth", [])?;

    tx.execute(
        "CREATE VIEW v_auth (
            token_hash,
            account_id,
            expiry
        ) AS SELECT
            lower(hex(account_id)),
            token_hash,
            expiry
        FROM auth",
        [],
    )?;

    tx.commit()?;

    Ok(())
}

impl Token {
    pub fn from_hex_string(s: String) -> Result<Token, AuthError> {
        if s.len() != 256 {
            Err(AuthError::TokenLengthError)
        } else {
            let bytes: Result<Vec<u8>, std::num::ParseIntError> = (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.into()))
                .collect();
            match bytes {
                Ok(b) => Ok(Token { bytes: b }),
                Err(e) => Err(e.into()),
            }
        }
    }

    pub fn new() -> Token {
        Token {
            bytes: (0..128).map(|_| rand::random::<u8>()).collect(),
        }
    }
}

impl From<std::num::ParseIntError> for AuthError {
    fn from(err: std::num::ParseIntError) -> Self {
        AuthError::TokenParseError(err)
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut s = String::with_capacity(self.bytes.len() * 2);
        for byte in &self.bytes {
            write!(s, "{:02X}", byte).unwrap();
        }
        write!(f, "{}", s)
    }
}

pub fn create_token(con: &mut Connection, account_id: &Uuid) -> Result<TokenCreatedResult, AuthError> {
    let tx = con.transaction_with_behavior(rusqlite::TransactionBehavior::Exclusive)?;

    let token = Token::new();
    let expiry = OffsetDateTime::now_utc() + Duration::days(30);

    let token_hash = bcrypt::hash(&token.bytes, 10)?;

    match tx.execute(
        "INSERT INTO auth (token_hash, account_id, expiry) VALUES (?, ?, ?)",
        params![token_hash, account_id, expiry],
    ) {
        Ok(_) => (),
        Err(err) => return Err(err.into()),
    };

    tx.commit()?;

    Ok(TokenCreatedResult {
        token: token.to_string(),
        account_id: account_id.to_owned(),
        expiry: expiry,
    })
}

pub fn validate_token(con: &mut Connection, token_str: String, account_id: &Uuid) -> Result<bool, AuthError> {
    let provided_token = Token::from_hex_string(token_str)?;

    let mut stmt = con.prepare("SELECT token_hash, expiry FROM auth WHERE account_id = (?)")?;
    let mut rows = stmt.query(params![account_id])?;

    let mut checked = 0;

    while let Some(row) = rows.next()? {
        let hash: String = row.get(0)?;
        let expiry: OffsetDateTime = row.get(1)?;

        let valid = bcrypt::verify(&provided_token.bytes, hash.as_str())?;
        if valid {
            if OffsetDateTime::now_utc() > expiry {
                return Err(AuthError::TokenExpired);
            }
            return Ok(true);
        }
        checked += 1;
    }

    match checked {
        0 => Err(AuthError::TokenDoesNotExist),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_new() {
        let t1 = Token::new();
        let t2 = Token::new();
        assert_eq!(t1.bytes.len(), 128);
        assert_ne!(t1, t2);
    }

    #[test]
    fn test_token_to_string() {
        let s = Token::new().to_string();
        println!("{}", s);
        assert_eq!(s.len(), 256);
    }

    #[test]
    fn test_token_from_string() {
        let t = Token::new();
        let s = t.to_string();
        let t2 = Token::from_hex_string(s).expect("should have converted");
        assert_eq!(t, t2);
    }

    #[test]
    fn test_token_from_string_invalid_length() {
        let result = Token::from_hex_string("BLAHBLAH".to_string());
        assert_eq!(result, Err(AuthError::TokenLengthError));
    }

    #[test]
    fn test_token_from_string_invalid_encoding() {
        let tok = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAX".to_string();
        let result = Token::from_hex_string(tok);
        assert!(result.is_err());
        assert_eq!(
            format!("{:?}", result),
            "Err(TokenParseError(ParseIntError { kind: InvalidDigit }))"
        ); // FIXME: This is horrid, there must be a better way
    }

    #[test]
    fn test_create_token() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_auth_table(&mut con).unwrap();

        let account_id = Uuid::new_v4();
        let tok = create_token(&mut con, &account_id).unwrap();

        assert_eq!(tok.account_id, account_id);
        assert_eq!(tok.token.len(), 256);
        assert!(tok.expiry > (OffsetDateTime::now_utc() + Duration::days(29)));
    }

    #[test]
    fn test_validate_token() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_auth_table(&mut con).unwrap();

        let account_id = Uuid::new_v4();
        let valid_token = create_token(&mut con, &account_id).unwrap();

        let res = validate_token(&mut con, valid_token.token.to_string(), &valid_token.account_id);
        assert_eq!(res.expect("should have a result"), true);

        let res = validate_token(&mut con, valid_token.token.to_string(), &Uuid::new_v4());
        assert_eq!(res, Err(AuthError::TokenDoesNotExist));

        // New token against different account
        let new_account_id = Uuid::new_v4();
        let new_token = create_token(&mut con, &new_account_id).unwrap();
        let res = validate_token(&mut con, new_token.token.to_string(), &account_id);
        assert_eq!(res, Ok(false));
    }

    #[test]
    fn test_validate_token_expired() {
        let mut con = rusqlite::Connection::open_in_memory().unwrap();
        create_auth_table(&mut con).unwrap();

        let token = Token::new();
        let token_hash = bcrypt::hash(token.bytes.to_owned(), 4).expect("");
        let account_id = Uuid::new_v4();
        let expiry = OffsetDateTime::now_utc() - Duration::days(1);

        con.execute(
            "INSERT INTO auth (token_hash, account_id, expiry) VALUES (?, ?, ?)",
            params![token_hash, account_id, expiry],
        )
        .unwrap();

        let res = validate_token(&mut con, token.to_string(), &account_id);
        assert_eq!(res, Err(AuthError::TokenExpired));
    }
}
