//! # Records Query
//!
//! The records query endpoint handles `RecordsQuery` messages — requests
//! to query the [`MessageStore`] for matching [`Write`] (and possibly
//! [`Delete`]) messages.

use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::authorization::Authorization;
use crate::endpoint::{Message, Reply, Status};
use crate::grants::Grant;
use crate::provider::{MessageStore, Provider};
use crate::records::{RecordsFilter, Write, protocol};
use crate::store::{self, Cursor, Pagination, RecordsQueryBuilder, Sort};
use crate::utils::cid;
use crate::{Descriptor, Result, forbidden, unexpected, utils};

/// Handle — or process — a [`Query`] message.
///
/// # Errors
///
/// The endpoint will return an error when message authorization fails or when
/// an issue occurs querying the [`MessageStore`].
pub async fn handle(
    owner: &str, query: Query, provider: &impl Provider,
) -> Result<Reply<QueryReply>> {
    query.validate()?;

    let store_query = if query.only_published() {
        // correct filter when querying soley for published records
        let mut query = query;
        query.descriptor.filter.published = Some(true);
        store::Query::from(query)
    } else {
        query.authorize(owner, provider).await?;
        let Some(authzn) = &query.authorization else {
            return Err(forbidden!("missing authorization"));
        };

        if authzn.author()? == owner {
            store::Query::from(query)
        } else {
            query.into_non_owner()?
        }
    };

    // fetch records matching query criteria
    let (records, cursor) = MessageStore::query(provider, owner, &store_query).await?;

    // short-circuit when no records found
    if records.is_empty() {
        return Ok(Reply {
            status: Status {
                code: StatusCode::OK.as_u16(),
                detail: None,
            },
            body: None,
        });
    }

    // build reply
    let mut entries = vec![];

    for record in records {
        let write: Write = record.try_into()?;

        // short-circuit when the record is an initial write
        if write.is_initial()? {
            entries.push(QueryReplyEntry {
                write,
                initial_write: None,
            });
            continue;
        }

        // get the initial write for the returned `RecordsWrite`
        let query = RecordsQueryBuilder::new()
            .add_filter(RecordsFilter::new().record_id(&write.record_id))
            .include_archived(true)
            .build();
        let (results, _) = MessageStore::query(provider, owner, &query).await?;
        let mut initial_write: Write = (&results[0]).try_into()?;
        initial_write.encoded_data = None;

        entries.push(QueryReplyEntry {
            write,
            initial_write: Some(initial_write),
        });
    }

    Ok(Reply {
        status: Status {
            code: StatusCode::OK.as_u16(),
            detail: None,
        },
        body: Some(QueryReply {
            entries: Some(entries),
            cursor,
        }),
    })
}

/// The [`Query`] message expected by the handler.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Query {
    /// The Query descriptor.
    pub descriptor: QueryDescriptor,

    /// The message authorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<Authorization>,
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
        self.authorization.as_ref()
    }

    async fn handle(self, owner: &str, provider: &impl Provider) -> Result<Reply<Self::Reply>> {
        handle(owner, self, provider).await
    }
}

/// [`QueryReply`] is returned by the handler in the [`Reply`] `body` field.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReply {
    /// Query reply entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<QueryReplyEntry>>,

    /// Pagination cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<Cursor>,
}

/// [`QueryReplyEntry`] represents a [`Write`] entry returned by the query.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryReplyEntry {
    /// The `RecordsWrite` message of the record if record exists.
    #[serde(flatten)]
    pub write: Write,

    /// The initial write of the record if the returned `RecordsWrite` message
    /// itself is not the initial write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_write: Option<Write>,
}

impl Query {
    async fn authorize(&self, owner: &str, provider: &impl Provider) -> Result<()> {
        let Some(authzn) = &self.authorization else {
            return Err(forbidden!("missing authorization"));
        };

        // verify grant
        if let Some(delegated_grant) = &authzn.author_delegated_grant {
            let grant: Grant = delegated_grant.try_into()?;
            grant.permit_query(&authzn.author()?, &authzn.signer()?, self, provider).await?;
        }

        // verify protocol when request invokes a protocol role
        if authzn.payload()?.protocol_role.is_some() {
            let Some(protocol) = &self.descriptor.filter.protocol else {
                return Err(unexpected!("missing protocol"));
            };
            let Some(protocol_path) = &self.descriptor.filter.protocol_path else {
                return Err(unexpected!("missing `protocol_path`"));
            };
            if protocol_path.contains('/') && self.descriptor.filter.context_id.is_none() {
                return Err(unexpected!("missing `context_id`"));
            }

            // verify protocol role is authorized
            let verifier = protocol::Authorizer::new(protocol)
                .context_id(self.descriptor.filter.context_id.as_ref());
            return verifier.permit_query(owner, self, provider).await;
        }

        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if let Some(protocol) = &self.descriptor.filter.protocol {
            utils::uri::validate(protocol)?;
        }

        if let Some(schema) = &self.descriptor.filter.schema {
            utils::uri::validate(schema)?;
        }

        let Some(published) = self.descriptor.filter.published else {
            return Ok(());
        };
        if published {
            return Ok(());
        }

        if self.descriptor.date_sort == Some(Sort::PublishedAsc)
            || self.descriptor.date_sort == Some(Sort::PublishedDesc)
        {
            return Err(unexpected!(
                "cannot sort by `date_published` when querying for unpublished records"
            ));
        }

        Ok(())
    }

    // when the `published` flag is unset and the query uses published-related
    // settings, set the `published` flag to true
    fn only_published(&self) -> bool {
        if let Some(published) = self.descriptor.filter.published {
            return published;
        }
        if self.descriptor.filter.date_published.is_some() {
            return true;
        }
        if self.descriptor.date_sort == Some(Sort::PublishedAsc)
            || self.descriptor.date_sort == Some(Sort::PublishedDesc)
        {
            return true;
        }
        if self.authorization.is_none() {
            return true;
        }
        false
    }

    // when requestor (message author) is not web node owner,
    // recreate filters to include query author as record author or recipient
    fn into_non_owner(self) -> Result<store::Query> {
        // let mut store_query = RecordsQueryBuilder::from(self.clone());
        let mut store_query = RecordsQueryBuilder::new();
        if let Some(date_sort) = self.descriptor.date_sort {
            store_query = store_query.sort(date_sort);
        }
        if let Some(pagination) = self.descriptor.pagination {
            store_query = store_query.pagination(pagination);
        }

        let Some(authzn) = &self.authorization else {
            return Err(forbidden!("missing authorization"));
        };
        let author = authzn.author()?;

        // New filter: copy query filter  and set `published` to true
        if self.descriptor.filter.published.is_none() {
            let filter = self.descriptor.filter.clone();
            store_query = store_query.add_filter(filter.published(true));
        }

        // New filter: copy query filter remove authors except `author`
        let mut filter = self.descriptor.filter.clone();
        filter.author = None;
        store_query = store_query.add_filter(filter.add_author(&author).published(false));

        // New filter: copy query filter and remove recipients except author
        let mut filter = self.descriptor.filter.clone();
        filter.recipient = None;
        store_query = store_query.add_filter(filter.add_recipient(&author).published(false));

        // New filter: author can query any record when authorized by a role
        if authzn.payload()?.protocol_role.is_some() {
            let mut filter = self.descriptor.filter.clone();
            filter.published = Some(false);
            store_query = store_query.add_filter(filter.published(false));
        }

        Ok(store_query.build())
    }
}

/// The [`Query`] message descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryDescriptor {
    /// The base descriptor
    #[serde(flatten)]
    pub base: Descriptor,

    /// Filter Records for query.
    pub filter: RecordsFilter,

    /// Specifies how dates should be sorted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_sort: Option<Sort>,

    /// The pagination cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
}
