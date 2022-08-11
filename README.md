# CapyCoin! 🪙

✨ A toy implementation of a non-distributed fixed circulation currency system! ✨

_Using the power of Rust and SQLite!_

## Features!

 * Create an account (and receive an access token)
 * Get account information (funds, creation date)
 * Get transactions (to/from, amount, fee, transaction date)
 * Get a single transaction information
 * Create a transaction

## ⚠️ Warning!

This is a toy project, which I used to learn Rust. There are **no guarantees of any security**,
and it has not been subject to security review.

There has also been no optimisation in terms of transaction volume, scaling, etc.

## Economics

The system is a closed circulation monetary system. The coin is non-divisible.

There is a "seed" account `4e9b616a-f11e-48b6-8c2f-9534d482e48e` which has 100,000 coin
when the database is created.

There is no "mint coin" functionality, but it would be trivial to add.

## System Design

### Core

The "core" of CapyCoin is a single threaded process which uses a SQLite database to hold accounts
and record transactions.

It listens for messages on a Unix (datagram) socket. Any clients which connect to this socket are
considered "trusted", so the intention is that the gateway and other internal applications connect
to provide user services.

### Gateway

The "gateway" is the user facing component of CapyCoin. This provides provision and validation
of access tokens which are associated with an account and have a expiry of 30 days.

This gateway provides a simple REST API which clients can use to interact with the core.

The gateway is threaded, as the token validation uses `bcrypt` which is an algorithm that
is intentionally slow to compute in order to make brute force attacks impractical.

The **seed account** is not accessible from the gateway service as it has no provisioned
access token.

## Get Started!

### Get the services running

 1. Install rust (eg: via `rustup`)
 2. `cargo build`
 3. `cargo run` (run the core)
 4. `cargo run --bin client_gateway` (run the client gateway, in a separate terminal)

## REST API Example

(In the following examples, the gateway is listening on `http://localhost:8000`)

Clients should set `Authentication: Bearer {TOKEN}` for all endpoints, except account creation.

We will use the account ID `ab62ad1e-eeaa-406a-acdb-4a43c63fda25` for these examples. These IDs are randomly generated v4 UUIDs. The dashes between the characters are optional.

### Create an Account

NOTE: When created, accounts have no funds.

#### Request (No payload):

```
POST /account/create
```

#### Response (JSON):

```
{
	"account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
	"token": "42A4EB6E8AEE8F83894CC7279AE6E066321DE213748BFB07FBEEE8072D0A95B7C6F9BD5F6281FA5D8D6819F74F94963C20F7711D0E517576625CDEA8D559DAA18707C6DC6523677B21EA9CCD9D41A01B3B833A115A4D878C73083AD6C7D34F851C9BEF835B28194EB6C40ACD36AE94366B6CECFD5AB85FFD3666DA200E5BD1E8",
	"token_expiry": "2022-09-14T13:24:23.136285Z"
}
```

### Get Account Information

#### Request (No payload):

```
GET /account/ab62ad1e-eeaa-406a-acdb-4a43c63fda25
```

#### Response (JSON):

```
{
	"account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
	"created": "2022-08-15T13:24:23.095609Z",
	"funds": 1000
}
```

### Create a transaction

#### Request (JSON):

```
POST /account/ab62ad1e-eeaa-406a-acdb-4a43c63fda25/transaction
```

```
{
	"receiver": "4e9b616af11e48b68c2f9534d482e48e",
	"amount": 1000
}
```

#### Response (JSON)

```
{
	"transaction_id": "d7ab6eee-93c6-48db-8ba8-e1246c923c0d",
	"timestamp": "2022-08-15T13:36:36.257736Z",
	"sender_account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
	"receiver_account_id": "4e9b616a-f11e-48b6-8c2f-9534d482e48e",
	"sender_account_funds": 900,
	"fee": 0
}
```

### Get Transactions

#### Request (No payload):

```
GET /account/ab62ad1e-eeaa-406a-acdb-4a43c63fda25/transactions?start=2000-01-01&end=3000-01-01
```

Parameters:

 * `start` (Optional) a date (`YYYY-MM-DD`) to select transactions from this date
 * `end` (Optional) a date (`YYY-MM-DD`) to select transactions up to this date

#### Response (JSON):

```
{
	"account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
	"transactions": [
		{
			"transaction_id": "d7ab6eee-93c6-48db-8ba8-e1246c923c0d",
			"timestamp": "2022-08-15T13:36:36.257736Z",
			"sender_account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
			"receiver_account_id": "4e9b616a-f11e-48b6-8c2f-9534d482e48e",
			"amount": 100,
			"fee": 0
		}
	]
}
```

### Get a single Transaction

#### Request (No payload):

```
GET /account/ab62ad1e-eeaa-406a-acdb-4a43c63fda25/transaction/d7ab6eee-93c6-48db-8ba8-e1246c923c0d
```

#### Response (JSON):

```
{
	"transaction_id": "d7ab6eee-93c6-48db-8ba8-e1246c923c0d",
	"timestamp": "2022-08-15T13:36:36.257736Z",
	"sender_account_id": "ab62ad1e-eeaa-406a-acdb-4a43c63fda25",
	"receiver_account_id": "4e9b616a-f11e-48b6-8c2f-9534d482e48e",
	"amount": 100,
	"fee": 0
}
```


## TODO

 * Client signing of transactions (ED25519). This would necessitate a client providing their public key
   at account creation time.

 * Transaction fees. I had a bit of a fun idea that any fees could go into a "fee account", and
   there could be a special service which pays out all active user accounts a stipend every day?

## Licence

This code is licenced under LGPLv3.
