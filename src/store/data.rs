//! Data record handling.

use std::io::{Cursor, Read, Write};
use std::str::FromStr;

use cid::Cid;
use ipld_core::ipld::Ipld;

use crate::provider::BlockStore;
use crate::store::block;
use crate::{Result, unexpected};

/// The maximum size of a message.
pub const MAX_ENCODED_SIZE: usize = 30000;

/// The maximum size of a block.
pub const CHUNK_SIZE: usize = 16;

/// Put a data record into the block store.
pub async fn put(
    owner: &str, data_cid: &str, reader: impl Read, store: &impl BlockStore,
) -> Result<(String, usize)> {
    let mut links = vec![];
    let mut byte_count = 0;
    let mut reader = reader;

    // read data stream in chunks, storing each chunk as an IPLD block
    loop {
        let mut buffer = [0u8; CHUNK_SIZE];
        if let Ok(bytes_read) = reader.read(&mut buffer[..]) {
            if bytes_read == 0 {
                break;
            }
            // encode buffer to IPLD block
            let ipld = Ipld::Bytes(buffer[..bytes_read].to_vec());
            let block = block::Block::encode(&ipld)?;

            // insert into the blockstore
            let cid = block.cid();
            store
                .put(owner, cid, block.data())
                .await
                .map_err(|e| unexpected!("issue storing data: {e}"))?;

            // save link to block
            let cid = Cid::from_str(cid).map_err(|e| unexpected!("issue parsing CID: {e}"))?;
            links.push(Ipld::Link(cid));
            byte_count += bytes_read;
        }
    }

    // create a root block linking to the data blocks
    let block = block::Block::encode(&Ipld::List(links))?;
    store.put(owner, data_cid, block.data()).await?;

    Ok((block.cid().to_string(), byte_count))
}

/// Get a data record from the block store.
pub async fn get(
    owner: &str, data_cid: &str, store: &impl BlockStore,
) -> Result<Option<impl Read>> {
    // get root block
    let Some(bytes) = store.get(owner, data_cid).await? else {
        return Ok(None);
    };

    // the root blook contains a list of links to data blocks
    let Ipld::List(links) = block::decode(&bytes)? else {
        return Ok(None);
    };

    // TODO: optimize by streaming the data blocks as fetched
    // fetch each data block
    let mut buf = Cursor::new(vec![]);

    for link in links {
        // get data block
        let Ipld::Link(link_cid) = link else {
            return Err(unexpected!("invalid link"));
        };
        let Some(bytes) = store.get(owner, &link_cid.to_string()).await? else {
            return Ok(None);
        };

        // get data block's payload
        let ipld_bytes = block::decode(&bytes)?;
        let Ipld::Bytes(bytes) = ipld_bytes else {
            return Ok(None);
        };

        buf.write_all(&bytes)?;
    }

    buf.set_position(0);
    Ok(Some(buf))
}
