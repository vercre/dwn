//! # Index Store
//!
//! The index store is responsible for storing and retrieving message indexes.

#![allow(dead_code)]
#![allow(unused_variables)]

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::provider::BlockStore;
use crate::store::{Entry, Query, block};

pub async fn insert(owner: &str, entry: &Entry, store: &impl BlockStore) -> Result<()> {
    let message_cid = entry.cid()?;
    let write = entry.as_write().unwrap();
    let fields = write.indexes();

    let indexes = IndexesBuilder::new().owner(owner).store(store).build();

    for (field, value) in fields {
        let mut index = indexes.get(&field).await?;
        index.insert(value.to_string(), &message_cid);
        indexes.update(index).await?;
    }

    Ok(())
}

pub async fn query(owner: &str, query: &Query, store: &impl BlockStore) -> Result<Vec<Entry>> {
    let indexes = IndexesBuilder::new().owner(owner).store(store).build();

    let Query::Records(rq) = query else {
        return Err(anyhow!("unsupported query type"));
    };

    let filter = &rq.filters[0];
    let mut entries = Vec::new();

    if let Some(published) = &filter.published {
        let index = indexes.get("published").await?;

        for (value, message_cid) in index.values {
            if value == "true" {
                let bytes = store.get(owner, &message_cid).await?.unwrap();
                let entry = block::decode(&bytes)?;
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

#[derive(Serialize)]
struct Cid(String);

pub struct Indexes<'a, S: BlockStore> {
    owner: &'a str,
    store: &'a S,
}

impl<S: BlockStore> Indexes<'_, S> {
    /// Get an index.
    pub async fn get(&self, field: &str) -> Result<Index> {
        let index_cid = block::compute_cid(&Cid(format!("{}-{}", self.owner, field)))?;

        // get the index block or return empty index
        let Some(data) = self.store.get(self.owner, &index_cid).await? else {
            return Ok(Index::new(field));
        };
        block::decode(&data)
    }

    /// Update an index.
    pub async fn update(&self, index: Index) -> Result<()> {
        let index_cid = block::compute_cid(&Cid(format!("{}-{}", self.owner, index.field)))?;

        // update the index block
        self.store.delete(self.owner, &index_cid).await?;
        self.store.put(self.owner, &index_cid, &block::encode(&index)?).await
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    field: String,
    values: BTreeMap<String, String>,
}

impl Index {
    pub fn new(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            values: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, value: impl Into<String>, message_cid: impl Into<String>) {
        self.values.insert(value.into(), message_cid.into());
    }
}

pub struct IndexesBuilder<O, S> {
    owner: O,
    indexes: Option<BTreeMap<String, String>>,
    store: S,
}

/// Store not set on IndexesBuilder.
#[doc(hidden)]
pub struct NoOwner;
/// Store has been set on IndexesBuilder.
#[doc(hidden)]
pub struct Owner<'a>(&'a str);

/// Store not set on IndexesBuilder.
#[doc(hidden)]
pub struct NoStore;
/// Store has been set on IndexesBuilder.
#[doc(hidden)]
pub struct Store<'a, S: BlockStore>(&'a S);

impl IndexesBuilder<NoOwner, NoStore> {
    pub const fn new() -> Self {
        Self {
            owner: NoOwner,
            indexes: None,
            store: NoStore,
        }
    }
}

impl<S> IndexesBuilder<NoOwner, S> {
    pub fn owner(self, owner: &str) -> IndexesBuilder<Owner, S> {
        IndexesBuilder {
            owner: Owner(owner),
            indexes: None,
            store: self.store,
        }
    }
}

impl<O> IndexesBuilder<O, NoStore> {
    pub fn store<S: BlockStore>(self, store: &S) -> IndexesBuilder<O, Store<'_, S>> {
        IndexesBuilder {
            owner: self.owner,
            indexes: self.indexes,
            store: Store(store),
        }
    }
}

impl<'a, S: BlockStore> IndexesBuilder<Owner<'a>, Store<'a, S>> {
    pub fn build(self) -> Indexes<'a, S> {
        Indexes {
            owner: self.owner.0,
            store: self.store.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use blockstore::{Blockstore as _, InMemoryBlockstore};
    use dwn_test::key_store::{self, ALICE_DID};

    use super::*;
    use crate::clients::records::WriteBuilder;
    use crate::store::{RecordsFilter, RecordsQuery};
    // use crate::data::MAX_ENCODED_SIZE;
    // use crate::store::block;

    #[tokio::test]
    async fn test_index() {
        let block_store = BlockStoreImpl::new();
        let alice_signer = key_store::signer(ALICE_DID);

        let write = WriteBuilder::new().published(true).sign(&alice_signer).build().await.unwrap();
        let entry = Entry::from(&write);

        let message_cid = entry.cid().unwrap();
        println!("put: {:?}", message_cid);

        let block = block::encode(&entry).unwrap();
        block_store.put(ALICE_DID, &message_cid, &block).await.unwrap();

        super::insert(ALICE_DID, &entry, &block_store).await.unwrap();

        let query = Query::Records(RecordsQuery {
            filters: vec![RecordsFilter {
                published: Some(true),
                ..Default::default()
            }],
            ..Default::default()
        });

        let entries = super::query(ALICE_DID, &query, &block_store).await.unwrap();
        println!("{:?}", entries);
    }

    struct BlockStoreImpl {
        blockstore: InMemoryBlockstore<64>,
    }

    impl BlockStoreImpl {
        pub fn new() -> Self {
            Self {
                blockstore: InMemoryBlockstore::<64>::new(),
            }
        }
    }

    impl BlockStore for BlockStoreImpl {
        async fn put(&self, owner: &str, cid: &str, data: &[u8]) -> Result<()> {
            // HACK: convert libipld CID to blockstore CID
            let block_cid = cid::Cid::from_str(cid)?;
            self.blockstore.put_keyed(&block_cid, data).await.map_err(Into::into)
        }

        async fn get(&self, owner: &str, cid: &str) -> Result<Option<Vec<u8>>> {
            // HACK: convert libipld CID to blockstore CID
            let block_cid = cid::Cid::try_from(cid)?;
            let Some(bytes) = self.blockstore.get(&block_cid).await? else {
                return Ok(None);
            };
            Ok(Some(bytes))
        }

        async fn delete(&self, owner: &str, cid: &str) -> Result<()> {
            let cid = cid::Cid::from_str(cid)?;
            self.blockstore.remove(&cid).await?;
            Ok(())
        }

        async fn purge(&self) -> Result<()> {
            unimplemented!()
        }
    }
}
