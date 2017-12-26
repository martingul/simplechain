use std::fs::{self, File};
use std::io::prelude::*;
use std::io::ErrorKind;
use std::path::Path;
use bincode::{serialize, deserialize, Infinite};
use sha2::{Sha256, Digest};
use rusqlite::Connection;
use base58::{FromBase58, ToBase58};
use hex::{FromHex, ToHex};
use secp256k1;
use secp256k1::key::{SecretKey, PublicKey};

use errors::CoreError;
use utils;

// TODO FIXME fix struct privacy (too many pub)

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct TransactionContent {
    pub sender_addr: Vec<u8>,
    pub sender_pubkey: Vec<u8>,
    pub receiver_addr: Vec<u8>,
    pub amount: i32,
    pub timestamp: i64
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct TransactionSigned {
    pub content: TransactionContent,
    signature: Vec<u8>
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Transaction {
    pub id: Vec<u8>,
    pub transaction: TransactionSigned // bad field name...
}

impl TransactionContent {
    // sign a transaction using schnorr signature
    pub fn get_signature(
        &self,
        private_key: SecretKey
    ) -> Result<Vec<u8>, CoreError> {
        println!("SIGN TRANSACTION");

        let secp = secp256k1::Secp256k1::new();
        // serialize the tx content
        let tx_content_encoded: Vec<u8> = serialize(&self, Infinite)?;

        // hash the tx content
        let mut hasher = Sha256::new();
        hasher.input(&tx_content_encoded);
        let tx_content_hashed = hasher.result();

        // create the input message with the hashed tx content
        let input = secp256k1::Message::from_slice(tx_content_hashed.as_slice())?;

        // return the signature created with the input message and private key
        Ok(secp.sign_schnorr(&input, &private_key)?.serialize())
    }
}

impl TransactionSigned {
    // hash a transaction to create its id
    pub fn get_id(&self) -> Result<Vec<u8>, CoreError> {
        // serialize the signed tx
        let tx_signed_encoded: Vec<u8> = serialize(&self, Infinite)?;

        // hash everything to return the id
        let mut hasher = Sha256::new();
        hasher.input(&tx_signed_encoded);
        Ok(hasher.result().as_slice().to_vec())
    }
}

impl Transaction {
    // create a transaction from raw bytes
    pub fn from_bytes(data: &Vec<u8>) -> Result<Transaction, CoreError> {
        // read data and deserialize into a Transaction struct
        let tx: Transaction = deserialize(&data[..])?;
        Ok(tx)
    }

    // verify a transaction using the signature and the public key
    pub fn verify(&self) -> Result<bool, CoreError> {
        println!("VERIFY TRANSACTION");

        let secp = secp256k1::Secp256k1::new();
        // serialize the tx content
        let tx_encoded: Vec<u8> = serialize(&self.transaction.content, Infinite)?;

        // hash the tx content
        let mut hasher = Sha256::new();
        hasher.input(&tx_encoded);
        let tx_hashed = hasher.result();

        // create the input message using the hashed tx content
        let input = secp256k1::Message::from_slice(tx_hashed.as_slice())?;

        // retrieve sig and pbkey from the tx
        let signature = secp256k1::schnorr::Signature::deserialize(&self.transaction.signature);
        let public_key = PublicKey::from_slice(
            &secp, &self.transaction.content.sender_pubkey
        )?;

        // verify the input message using the signature and pbkey
        Ok(
            match secp.verify_schnorr(&input, &signature, &public_key) {
                Ok(()) => true,
                _ => false
            }
        )
    }

    // store a transaction on database (cache) for further block creation
    // TODO rewrite this with redis
    pub fn store_db(&self) -> Result<(), CoreError> {
        println!("STORE TRANSACTION [DB]");
        // TODO rewrite this with connection pools
        // TODO get the db address string from config.json
        let conn = Connection::open("db/storage.db")?;

        let id = &self.id.to_hex();
        let sender_addr = &self.transaction.content.sender_addr.to_base58();
        let sender_pubkey = &self.transaction.content.sender_pubkey.to_hex();
        let receiver_addr = &self.transaction.content.receiver_addr.to_base58();
        let amount = &self.transaction.content.amount;
        let timestamp = &self.transaction.content.timestamp;
        let signature = &self.transaction.signature.to_hex();

        conn.execute("INSERT INTO transactions(
            id, sender_addr, sender_pubkey, receiver_addr, amount, timestamp, signature
        ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            &[&*id, &*sender_addr, &*sender_pubkey, &*receiver_addr, &*amount, &*timestamp, &*signature])?;

        Ok(())
    }
}

// create a transaction, sign it, hash it and return it
pub fn new(
    sender_privkey: SecretKey,
    sender_pubkey: Vec<u8>,
    sender_addr: Vec<u8>,
    receiver_addr: Vec<u8>,
    amount: i32
) -> Result<Transaction, CoreError> {
    println!("CREATE TRANSACTION");

    let timestamp: i64 = utils::get_current_timestamp();

    let tx_content = TransactionContent {
        sender_addr: sender_addr,
        sender_pubkey: sender_pubkey,
        receiver_addr: receiver_addr,
        amount: amount,
        timestamp: timestamp
    };

    // sign the current tx content
    let signature: Vec<u8> = tx_content.get_signature(sender_privkey)?;

    // create a signed tx with the signature
    let tx_signed = TransactionSigned {
        content: tx_content,
        signature: signature
    };

    // get the tx id (hash) using the signed tx content
    let id: Vec<u8> = tx_signed.get_id()?;

    //  TODO maybe rewrite this by removing the struct nesting
    // this will be easier for cross-language
    // Transaction {
    //     id: ...
    //     sender_addr: ...
    //     ...
    //     signature: ...
    // }
    // but be careful maybe recreating Transaction with TransactionContent's
    // and TransactionSigned's fields will make the signature obsolete

    // TEST
    // println!("id: {}", id.to_hex());
    // println!("sender_addr: {}", tx_signed.content.sender_addr.to_base58());
    // println!("sender_pubkey: {}", tx_signed.content.sender_pubkey.to_hex());
    // println!("receiver_addr: {}", tx_signed.content.receiver_addr.to_base58());
    // println!("amount: {}", tx_signed.content.amount);
    // println!("timestamp: {}", tx_signed.content.timestamp);
    // println!("signature: {}", tx_signed.signature.to_hex());

    // return the final tx
    Ok(Transaction {
        id: id,
        transaction: tx_signed
    })
}

// return a Transaction struct filled with given field values
pub fn from(
    id: &String,
    sender_addr: &String,
    sender_pubkey: &String,
    receiver_addr: &String,
    amount: i32,
    timestamp: i64,
    signature: &String,
) -> Result<Transaction, CoreError> {
    let id: Vec<u8> = FromHex::from_hex(id)?;
    let sender_addr: Vec<u8> = sender_addr.from_base58()?;
    let sender_pubkey: Vec<u8> = FromHex::from_hex(sender_pubkey)?;
    let receiver_addr: Vec<u8> = receiver_addr.from_base58()?;
    let signature: Vec<u8> = FromHex::from_hex(signature)?;

    Ok(Transaction {
        id: id,
        transaction: TransactionSigned {
            content: TransactionContent {
                sender_addr: sender_addr,
                sender_pubkey: sender_pubkey,
                receiver_addr: receiver_addr,
                amount: amount,
                timestamp: timestamp
            },
            signature: signature,
        },
    })
}

// read all cached database transactions
pub fn read_db() -> Result<Vec<Transaction>, CoreError> {
    println!("READ TRANSACTIONS [DB]");
    // TODO rewrite this with connection pools
    let conn = Connection::open("db/storage.db")?;

    let mut transactions: Vec<Transaction> = Vec::new();

    let mut stmt = conn.prepare(
        "SELECT id, sender_addr, sender_pubkey, receiver_addr, amount, timestamp, signature
        FROM transactions"
    )?;

    let rows = stmt.query_map(&[], |row| {
        let id: String = row.get(0);
        let sender_addr: String = row.get(1);
        let sender_pubkey: String = row.get(2);
        let receiver_addr: String = row.get(3);
        let amount: i32 = row.get(4);
        let timestamp: i64 = row.get(5);
        let signature: String = row.get(6);

        Transaction {
            id: id.into_bytes(),
            transaction: TransactionSigned {
                content: TransactionContent {
                    sender_addr: sender_addr.into_bytes(),
                    sender_pubkey: sender_pubkey.into_bytes(),
                    receiver_addr: receiver_addr.into_bytes(),
                    amount: amount,
                    timestamp: timestamp
                },
                signature: signature.into_bytes()
            }
        }
    })?;

    for tx in rows {
        transactions.push(tx?);
    }

    Ok(transactions)
}

// delete all cached transactions from database
pub fn clean_db() -> Result<(), CoreError> {
    println!("CLEAN TRANSACTIONS [DB]");
    // TODO rewrite this with connection pools
    // TODO get the db address string from config.json
    let conn = Connection::open("db/storage.db")?;

    conn.execute("DELETE FROM transactions", &[])?;
    Ok(())
}

// // store a transaction on disk (cache) for further block creation
// pub fn store_disk(tx: &Transaction) -> Result<(), CoreError> {
//     println!("STORE TRANSACTION [DISK]");
//     let tx_encoded: Vec<u8> = serialize(&tx, Infinite)?;
//     let tx_dir_path = Path::new("./transactions");
//
//     let ready: bool = match fs::create_dir(tx_dir_path) {
//         Ok(_) => true,
//         Err(e) => match e.kind() {
//             ErrorKind::AlreadyExists => true,
//             _ => false,
//         },
//     };
//
//     if ready {
//         let tx_dir = fs::read_dir(tx_dir_path)?;
//         let tx_file_path = tx_dir_path.join(format!("tx{}.bin", tx_dir.count() + 1));
//         let mut tx_file = File::create(tx_file_path)?;
//
//         tx_file.write_all(&tx_encoded)?;
//     }
//
//     Ok(())
// }
//
// // read all cached (on disk) transactions
// pub fn read_disk() -> Result<Vec<Transaction>, String> {
//     println!("READ TRANSACTIONS [DISK]");
//     let tx_dir_path = Path::new("./transactions");
//
//     let ready: bool = match fs::read_dir(tx_dir_path) {
//         Ok(_) => true,
//         Err(e) => false,
//     };
//
//     if ready {
//         let tx_dir = fs::read_dir(tx_dir_path)?;
//         let mut transactions: Vec<Transaction> = Vec::new();
//
//         for tx_file in tx_dir {
//             let mut tx_file = File::open(tx_file?.path())?;
//             let mut buffer = vec![0; 1024];
//
//             tx_file.read(&mut buffer);
//
//             let tx: Transaction = deserialize(&buffer[..])?;
//             transactions.push(tx);
//         }
//
//         Ok(transactions)
//     } else {
//         Err(String::from("Error"))
//     }
// }
//
// // delete all cached transactions from disk
// pub fn clean_disk() -> Result<(), CoreError> {
//     println!("CLEAN TRANSACTIONS [DISK]");
//     Ok(())
// }
