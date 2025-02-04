//! # Messages Query
//!
//! The messages query endpoint handles `MessagesQuery` messages — requests
//! to query the [`EventLog`] for matching persisted messages (of any type).

use http::StatusCode;
use serde::{Deserialize, Serialize};

use super::MessagesFilter;
use crate::authorization::Authorization;
use crate::endpoint::{Message, Reply, Status};
use crate::provider::{EventLog, Provider};
use crate::store::{self, Cursor};
use crate::utils::cid;
use crate::{Descriptor, Result, forbidden, grants};

/// Handle — or process — a [`Query`] message.
///
/// # Errors
///
/// The endpoint will return an error when message authorization fails or when
/// an issue occurs querying the [`EventLog`].
pub async fn handle(
    owner: &str, query: Query, provider: &impl Provider,
) -> Result<Reply<QueryReply>> {
    query.authorize(owner, provider).await?;

    let query = store::Query::from(query);
    let (events, cursor) = EventLog::query(provider, owner, &query).await?;

    let events = events.iter().map(|e| e.cid().unwrap_or_default()).collect::<Vec<String>>();
    let entries = if events.is_empty() { None } else { Some(events) };

    Ok(Reply {
        status: Status {
            code: StatusCode::OK.as_u16(),
            detail: None,
        },
        body: Some(QueryReply { entries, cursor }),
    })
}

/// The [`Query`] message expected by the handler.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Query {
    /// The `Query` descriptor.
    pub descriptor: QueryDescriptor,

    /// The message authorization.
    pub authorization: Authorization,
}

impl Message for Query {
    type Reply = QueryReply;

    fn cid(&self) -> Result<String> {
        cid::from_value(self)
    }

    fn descriptor(&self) -> &Descriptor {
        &self.descriptor.base
    }

    fn authorization(&self) -> Option<&Authorization> {
        Some(&self.authorization)
    }

    async fn handle(self, owner: &str, provider: &impl Provider) -> Result<Reply<Self::Reply>> {
        handle(owner, self, provider).await
    }
}

impl Query {
    async fn authorize(&self, owner: &str, provider: &impl Provider) -> Result<()> {
        let authzn = &self.authorization;

        let author = authzn.author()?;
        if author == owner {
            return Ok(());
        }

        // verify grant
        let Some(grant_id) = &authzn.payload()?.permission_grant_id else {
            return Err(forbidden!("author has no grant"));
        };
        let grant = grants::fetch_grant(owner, grant_id, provider).await?;
        grant.verify(owner, &authzn.signer()?, self.descriptor(), provider).await?;

        // verify filter protocol
        if grant.data.scope.protocol().is_none() {
            return Ok(());
        }

        let protocol = grant.data.scope.protocol();
        for filter in &self.descriptor.filters {
            if filter.protocol.as_deref() != protocol {
                return Err(forbidden!("filter and grant protocols do not match"));
            }
        }

        Ok(())
    }
}

/// [`QueryReply`] is returned by the handler in the [`Reply`] `body` field.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct QueryReply {
    /// Entries matching the message's query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<String>>,

    /// The message authorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Cursor>,
}

/// The [`Query`] message descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDescriptor {
    /// The base descriptor
    #[serde(flatten)]
    pub base: Descriptor,

    /// Filters to apply when querying for messages.
    pub filters: Vec<MessagesFilter>,

    /// The pagination cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Cursor>,
}
