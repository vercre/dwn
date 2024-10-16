//! # CID (Content Identifier)

use multihash_codetable::{Code, MultihashDigest};
use serde::Serialize;

const RAW: u64 = 0x55;

/// Compute a CID from provided payload.
pub fn compute_cid<T: Serialize>(payload: &T) -> anyhow::Result<String> {
    // serialize to CBOR
    let mut buf = Vec::new();
    ciborium::into_writer(payload, &mut buf)?;

    // hash
    let hash = Code::Sha2_256.digest(&buf);
    let cid = cid::Cid::new_v1(RAW, hash);

    Ok(cid.to_string())
}
