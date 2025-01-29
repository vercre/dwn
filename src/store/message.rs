//! # Message Store

use crate::provider::BlockStore;
use crate::store::{Cursor, Entry, Query, block, index};
use crate::{Result, unexpected};

/// Store a message in the underlying store.
pub async fn put(owner: &str, entry: &Entry, store: &impl BlockStore) -> Result<()> {
    // store entry block
    let message_cid = entry.cid()?;
    store.delete(owner, &message_cid).await?;
    store.put(owner, &message_cid, &block::encode(entry)?).await?;

    // index entry
    index::insert(owner, entry, store).await
}

/// Queries the underlying store for matches to the provided query.
// fn query(&self, owner: &str, query: &Query) -> impl Future<Output = Result<Vec<Entry>>> + Send;
pub async fn query(
    owner: &str, query: &Query, store: &impl BlockStore,
) -> Result<(Vec<Entry>, Option<Cursor>)> {
    let mut results = index::query(owner, query, store).await?;

    // return cursor when paging is used
    let limit = query.pagination.as_ref().map(|p| p.limit.unwrap_or_default()).unwrap_or_default();

    let cursor = if limit > 0 && limit < results.len() {
        let sort_field = query.sort.to_string();

        // set cursor to the last item remaining after the spliced result.
        results.pop().map(|item| Cursor {
            message_cid: item.message_cid.clone(),
            value: item.fields[&sort_field].clone(),
        })
    } else {
        None
    };

    let mut entries = Vec::new();
    for item in results {
        let Some(bytes) = store.get(owner, &item.message_cid).await? else {
            return Err(unexpected!("missing block for message cid"));
        };
        entries.push(block::decode(&bytes)?);
    }

    Ok((entries, cursor))
}

/// Fetch a single message by CID from the underlying store, returning
/// `None` if no message was found.
pub async fn get(
    owner: &str, message_cid: &str, store: &impl BlockStore,
) -> Result<Option<Entry>> {
    let Some(bytes) = store.get(owner, message_cid).await? else {
        return Ok(None);
    };
    Ok(Some(block::decode(&bytes)?))
}

/// Delete message associated with the specified id.
pub async fn delete(owner: &str, message_cid: &str, store: &impl BlockStore) -> Result<()> {
    index::delete(owner, message_cid, store).await?;
    store.delete(owner, message_cid).await.map_err(Into::into)
}
