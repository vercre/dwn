#![allow(missing_docs)]
#![allow(unused_variables)]

//! # Provider
//!
//! Implementation of the `Provider` trait for testing and examples.

pub mod data;
pub mod event;
pub mod message;
pub mod task;

use anyhow::{anyhow, Result};
use blockstore::InMemoryBlockstore;
use serde::Deserialize;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::opt::RecordId;
use surrealdb::Surreal;
use vercre_dwn::protocols::Configure;
use vercre_dwn::provider::{DidResolver, Document, KeyStore, Keyring, MessageStore, Provider};
use vercre_infosec::{Algorithm, Cipher, Signer};

use crate::keystore::{Keystore, OWNER_DID};

const NAMESPACE: &str = "integration-test";

#[derive(Clone)]
pub struct ProviderImpl {
    db: Surreal<Db>,
    blockstore: InMemoryBlockstore<64>,
}

impl Provider for ProviderImpl {}

impl ProviderImpl {
    pub async fn new() -> Result<Self> {
        // surreal db
        let db = Surreal::new::<Mem>(()).await?;
        db.use_ns(NAMESPACE).use_db(OWNER_DID).await?;

        // blockstore
        let blockstore = InMemoryBlockstore::<64>::new();

        let provider = Self { db, blockstore };

        // load a protocol configuration
        let bytes = include_bytes!("./store/protocol.json");
        let config: Configure = serde_json::from_slice(bytes).expect("should deserialize");
        MessageStore::put(&provider, OWNER_DID, &config).await?;

        Ok(provider)
    }
}

#[derive(Debug, Deserialize)]
struct Record {
    #[allow(dead_code)]
    id: RecordId,
}

impl DidResolver for ProviderImpl {
    async fn resolve(&self, url: &str) -> Result<Document> {
        serde_json::from_slice(include_bytes!("./store/did.json"))
            .map_err(|e| anyhow!(format!("issue deserializing document: {e}")))
    }
}

struct KeyStoreImpl(Keystore);

impl KeyStore for ProviderImpl {
    fn keyring(&self, _identifier: &str) -> Result<impl Keyring> {
        Ok(KeyStoreImpl(Keystore {}))
    }

    // fn signer(&self, _identifier: &str) -> Result<impl Signer> {
    //     Ok(KeyStoreImpl(Keystore {}))
    // }

    // fn cipher(&self, _identifier: &str) -> Result<impl Cipher> {
    //     Ok(KeyStoreImpl(Keystore {}))
    // }
}

impl Keyring for KeyStoreImpl {}

impl Signer for KeyStoreImpl {
    async fn try_sign(&self, msg: &[u8]) -> Result<Vec<u8>> {
        Keystore::try_sign(msg)
    }

    async fn public_key(&self) -> Result<Vec<u8>> {
        Keystore::public_key()
    }

    fn algorithm(&self) -> Algorithm {
        Keystore::algorithm()
    }

    fn verification_method(&self) -> String {
        Keystore::verification_method()
    }
}

impl Cipher for KeyStoreImpl {
    async fn encrypt(&self, _plaintext: &[u8], _recipient_public_key: &[u8]) -> Result<Vec<u8>> {
        todo!()
    }

    fn ephemeral_public_key(&self) -> Vec<u8> {
        todo!()
    }

    async fn decrypt(&self, _ciphertext: &[u8], _sender_public_key: &[u8]) -> Result<Vec<u8>> {
        todo!()
    }
}
