use time::OffsetDateTime;
use uuid::Uuid;

use crate::account::{Account, AccountCreatedResult, AccountError};
use crate::auth::AuthError;
use crate::ledger::{Transaction, TransactionError, TransactionReceipt};

use serde::{Deserialize, Serialize};

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    CreateNewAccount,

    GetAccount {
        account_id: Uuid,
    },

    GetAccounts,

    GetTransactions {
        account_id: Uuid,
        time_range_start: OffsetDateTime,
        time_range_end: OffsetDateTime,
    },

    GetTransaction {
        account_id: Uuid,
        transaction_id: Uuid,
    },

    CreateTransaction {
        sender_id: Uuid,
        receiver_id: Uuid,
        amount: i64,
    },
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct ClientMessagePacket {
    pub v: i32,
    pub message_id: Uuid,
    pub message: ClientMessage,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct ServerError {
    pub error_type: String,
    pub error_message: String,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub enum ServerMessage {
    CreateNewAccountResponse(Result<AccountCreatedResult, ServerError>),
    GetAccountResponse(Result<Account, ServerError>),
    GetAccountsResponse(Result<Vec<Account>, ServerError>),
    GetTransactionsResponse(Result<Vec<Transaction>, ServerError>),
    GetTransactionResponse(Result<Transaction, ServerError>),
    CreateTransactionResponse(Result<TransactionReceipt, ServerError>),
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
pub struct ServerMessagePacket {
    pub v: i32,
    pub message_id: Uuid,
    pub message: ServerMessage,
}

/// Implementation ///

// Client Message
impl ClientMessage {
    pub fn to_packet(self) -> ClientMessagePacket {
        ClientMessagePacket::new(self)
    }
}

// Client Message Packet
impl ClientMessagePacket {
    pub fn new(msg: ClientMessage) -> ClientMessagePacket {
        ClientMessagePacket {
            v: 1,
            message_id: Uuid::new_v4(),
            message: msg,
        }
    }

    pub fn to_json(&self) -> Option<String> {
        match serde_json::to_string(&self) {
            Ok(ser) => Some(ser),
            Err(err) => {
                eprintln!("Error serialising `ClientMessage` {:?}: {:?}", self, err);
                None
            }
        }
    }

    pub fn from_json(value: &str) -> Option<ClientMessagePacket> {
        match serde_json::from_str(value) {
            Ok::<ClientMessagePacket, _>(msg) => match msg.v {
                1 => Some(msg),
                _ => {
                    eprintln!("Unexpected message version!");
                    None
                }
            },
            Err(err) => {
                eprintln!("Error deserialising `ClientMessagePacket`: {:?}", err);
                None
            }
        }
    }
}

// Server Message
impl ServerMessage {
    pub fn to_packet(self) -> ServerMessagePacket {
        ServerMessagePacket::new(self)
    }

    // Create a ServerMessagePacket as a response for a ClientMessagePacket
    pub fn as_resp_for(self, msg: &ClientMessagePacket) -> ServerMessagePacket {
        ServerMessagePacket {
            v: 1,
            message_id: msg.message_id, // Same ID!
            message: self,
        }
    }
}

// Server Message Packet
impl ServerMessagePacket {
    // Create from ServerMessage
    pub fn new(msg: ServerMessage) -> ServerMessagePacket {
        ServerMessagePacket {
            v: 1,
            message_id: Uuid::new_v4(),
            message: msg,
        }
    }

    // Convert to JSON
    pub fn to_json(&self) -> Option<String> {
        match serde_json::to_string(&self) {
            Ok(ser) => Some(ser),
            Err(err) => {
                eprintln!("Error serialising `ServerMessagePacket` {:?}: {:?}", self, err);
                None
            }
        }
    }

    // Convert from JSON
    pub fn from_json(value: &str) -> Option<ServerMessagePacket> {
        match serde_json::from_str(value) {
            Ok::<ServerMessagePacket, _>(msg) => match msg.v {
                1 => Some(msg),
                _ => {
                    eprintln!("Unexpected message version!");
                    None
                }
            },
            Err(err) => {
                eprintln!("Error deserialising `ServerMessagePacket`: {:?}", err);
                None
            }
        }
    }
}

// Server Error //

impl From<AccountError> for ServerError {
    // Conversion for Serialisation
    fn from(error: AccountError) -> Self {
        ServerError {
            error_type: "AccountError".to_string(),
            error_message: format!("{:?}", error),
        }
    }
}

impl From<TransactionError> for ServerError {
    // Conversion for Serialisation
    fn from(error: TransactionError) -> Self {
        ServerError {
            error_type: "TransactionError".to_string(),
            error_message: format!("{:?}", error),
        }
    }
}

impl From<AuthError> for ServerError {
    fn from(error: AuthError) -> Self {
        ServerError {
            error_type: "AuthError".to_string(),
            error_message: format!("{:?}", error),
        }
    }
}

// ServerMessage (for function returns) //

// CreateNewAccountResponse
impl From<Result<AccountCreatedResult, AccountError>> for ServerMessage {
    fn from(result: Result<AccountCreatedResult, AccountError>) -> Self {
        match result {
            Ok(r) => ServerMessage::CreateNewAccountResponse(Ok(r)),
            Err(e) => ServerMessage::CreateNewAccountResponse(Err(e.into())),
        }
    }
}

// GetAccountResponse
impl From<Result<Account, AccountError>> for ServerMessage {
    fn from(result: Result<Account, AccountError>) -> Self {
        match result {
            Ok(r) => ServerMessage::GetAccountResponse(Ok(r)),
            Err(e) => ServerMessage::GetAccountResponse(Err(e.into())),
        }
    }
}

// GetAccountsResponse
impl From<Result<Vec<Account>, AccountError>> for ServerMessage {
    fn from(result: Result<Vec<Account>, AccountError>) -> Self {
        match result {
            Ok(r) => ServerMessage::GetAccountsResponse(Ok(r)),
            Err(e) => ServerMessage::GetAccountsResponse(Err(e.into())),
        }
    }
}

// GetTransactionsResponse
impl From<Result<Vec<Transaction>, TransactionError>> for ServerMessage {
    fn from(result: Result<Vec<Transaction>, TransactionError>) -> Self {
        match result {
            Ok(r) => ServerMessage::GetTransactionsResponse(Ok(r)),
            Err(e) => ServerMessage::GetTransactionsResponse(Err(e.into())),
        }
    }
}

// GetTransactionResponse
impl From<Result<Transaction, TransactionError>> for ServerMessage {
    fn from(result: Result<Transaction, TransactionError>) -> Self {
        match result {
            Ok(r) => ServerMessage::GetTransactionResponse(Ok(r)),
            Err(e) => ServerMessage::GetTransactionResponse(Err(e.into())),
        }
    }
}

// CreateTransactionResponse
impl From<Result<TransactionReceipt, TransactionError>> for ServerMessage {
    fn from(result: Result<TransactionReceipt, TransactionError>) -> Self {
        match result {
            Ok(r) => ServerMessage::CreateTransactionResponse(Ok(r)),
            Err(e) => ServerMessage::CreateTransactionResponse(Err(e.into())),
        }
    }
}
