//! # Read
//!
//! `Read` is a message type used to read a record in the web node.

use async_trait::async_trait;
use base64ct::{Base64UrlUnpadded, Encoding};
use chrono::{DateTime, Utc};
use http::StatusCode;
use serde::{Deserialize, Serialize};

use crate::auth::{Authorization, AuthorizationBuilder};
use crate::data::cid;
use crate::endpoint::{Message, Reply, Status};
use crate::permissions::{self, Protocol};
use crate::provider::{MessageStore, Provider, Signer};
use crate::records::{DataStream, DelegatedGrant, Delete, RecordsFilter, Write};
use crate::store::RecordsQuery;
use crate::{Descriptor, Error, Interface, Method, Result, forbidden, unexpected};

/// Process `Read` message.
///
/// # Errors
/// TODO: Add errors
pub async fn handle(owner: &str, read: Read, provider: &impl Provider) -> Result<Reply<ReadReply>> {
    // get the latest active `RecordsWrite` and `RecordsDelete` messages
    let query = RecordsQuery::from(read.clone()).build();
    let (entries, _) = MessageStore::query(provider, owner, &query).await?;
    if entries.is_empty() {
        return Err(Error::NotFound("no matching records found".to_string()));
    }
    if entries.len() > 1 {
        return Err(unexpected!("multiple messages exist"));
    }

    // if the matched message is a `RecordsDelete`, mark as not-found and return
    // both the RecordsDelete and the initial RecordsWrite
    if entries[0].descriptor().method == Method::Delete {
        // TODO: implement this

        //   let initial_write = await RecordsWrite.fetchInitialRecordsWriteMessage(this.messageStore, tenant, recordsDeleteMessage.descriptor.recordId);
        //   if initial_write.is_none() {
        //     return Err(unexpected!("Initial write for deleted record not found"));
        //   }

        //   // perform authorization before returning the delete and initial write messages
        //   const parsedInitialWrite = await RecordsWrite.parse(initial_write);
        //
        // if let Err(e)= RecordsReadHandler.authorizeRecordsRead(tenant, recordsRead, parsedInitialWrite, this.messageStore){
        //     // return messageReplyFromError(error, 401);
        //     return Err(e);
        // }
        //
        // return {
        //     status : { code: 404, detail: 'Not Found' },
        //     entry  : {
        //       recordsDelete: recordsDeleteMessage,
        //       initialWrite
        //     }
        // }
    }

    let mut write = Write::try_from(&entries[0])?;

    // TODO: review against the original code — it should take a store provider
    // verify the fetched message can be safely returned to the requestor
    read.authorize(owner, &write, provider).await?;

    let data = if let Some(encoded) = write.encoded_data {
        write.encoded_data = None;
        let buffer = Base64UrlUnpadded::decode_vec(&encoded)?;
        Some(DataStream::from(buffer))
    } else {
        DataStream::from_store(owner, &write.descriptor.data_cid, provider).await?
    };

    write.encoded_data = None;

    // attach initial write if latest RecordsWrite is not initial write
    let initial_write = if write.is_initial()? {
        None
    } else {
        let query = RecordsQuery::new().record_id(&write.record_id).include_archived(true).build();
        let (records, _) = MessageStore::query(provider, owner, &query).await?;
        if records.is_empty() {
            return Err(unexpected!("initial write not found"));
        }

        let Some(mut initial_write) = records[0].as_write().cloned() else {
            return Err(unexpected!("expected `RecordsWrite` message"));
        };

        initial_write.encoded_data = None;
        Some(initial_write)
    };

    Ok(Reply {
        status: Status {
            code: StatusCode::OK.as_u16(),
            detail: None,
        },
        body: Some(ReadReply {
            entry: ReadReplyEntry {
                records_write: Some(write.clone()),
                records_delete: None,
                initial_write,
                data,
            },
        }),
    })
}

/// Records read message payload
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    /// Read descriptor.
    pub descriptor: ReadDescriptor,

    /// Message authorization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<Authorization>,
}

#[async_trait]
impl Message for Read {
    type Reply = ReadReply;

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

/// Read reply.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReply {
    /// The read reply entry.
    pub entry: ReadReplyEntry,
}

/// Read reply.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReplyEntry {
    /// The latest `RecordsWrite` message of the record if record exists
    /// (not deleted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_write: Option<Write>,

    /// The `RecordsDelete` if the record is deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records_delete: Option<Delete>,

    /// The initial write of the record if the returned `RecordsWrite` message
    /// itself is not the initial write or if a `RecordsDelete` is returned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_write: Option<Write>,

    /// The data for the record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<DataStream>,
}

impl Read {
    async fn authorize(&self, owner: &str, write: &Write, store: &impl MessageStore) -> Result<()> {
        let Some(authzn) = &self.authorization else {
            return Ok(());
        };
        let author = authzn.author()?;

        // authorization not required for published data
        if write.descriptor.published.unwrap_or_default() {
            return Ok(());
        }

        // authorize delegate
        if let Some(delegated_grant) = &authzn.author_delegated_grant {
            let grant = delegated_grant.to_grant()?;
            grant.verify_scope(write)?;
        }

        // owner can read records they authored
        if author == owner {
            return Ok(());
        }

        // recipient can read
        if let Some(recipient) = &write.descriptor.recipient {
            if &author == recipient {
                return Ok(());
            }
        }

        // author can read
        if author == write.authorization.author()? {
            return Ok(());
        }

        // verify grant
        if let Some(grant_id) = &authzn.jws_payload()?.permission_grant_id {
            let grant = permissions::fetch_grant(owner, grant_id, store).await?;
            grant.permit_read(owner, &author, self, write, store).await?;
            return Ok(());
        }

        // verify protocol role and action
        if let Some(protocol) = &write.descriptor.protocol {
            let protocol = Protocol::new(protocol);
            protocol.permit_read(owner, self, store).await?;
            return Ok(());
        }

        Err(forbidden!("read cannot be authorized"))
    }
}

/// Reads read descriptor.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadDescriptor {
    /// The base descriptor
    #[serde(flatten)]
    pub base: Descriptor,

    /// Defines the filter for the read.
    pub filter: RecordsFilter,
}

/// Options to use when creating a permission grant.
#[derive(Clone, Debug, Default)]
pub struct ReadBuilder {
    message_timestamp: DateTime<Utc>,
    filter: RecordsFilter,
    permission_grant_id: Option<String>,
    protocol_role: Option<String>,
    delegated_grant: Option<DelegatedGrant>,
    authorize: Option<bool>,
}

impl ReadBuilder {
    /// Returns a new [`ReadBuilder`]
    #[must_use]
    pub fn new() -> Self {
        Self {
            message_timestamp: Utc::now(),
            ..Self::default()
        }
    }

    /// Specifies the permission grant ID.
    #[must_use]
    pub fn filter(mut self, filter: RecordsFilter) -> Self {
        self.filter = filter;
        self
    }

    // /// The datetime the record was created. Defaults to now.
    // #[must_use]
    // pub const fn message_timestamp(mut self, message_timestamp: DateTime<Utc>) -> Self {
    //     self.message_timestamp = Some(message_timestamp);
    //     self
    // }

    /// Specifies the permission grant ID.
    #[must_use]
    pub fn permission_grant_id(mut self, permission_grant_id: impl Into<String>) -> Self {
        self.permission_grant_id = Some(permission_grant_id.into());
        self
    }

    /// Specify a protocol role for the record.
    #[must_use]
    pub const fn authorize(mut self, authorize: bool) -> Self {
        self.authorize = Some(authorize);
        self
    }

    /// Specify a protocol role for the record.
    #[must_use]
    pub fn protocol_role(mut self, protocol_role: impl Into<String>) -> Self {
        self.protocol_role = Some(protocol_role.into());
        self
    }

    /// The delegated grant used with this record.
    #[must_use]
    pub fn delegated_grant(mut self, delegated_grant: DelegatedGrant) -> Self {
        self.delegated_grant = Some(delegated_grant);
        self
    }

    /// Build the write message.
    ///
    /// # Errors
    /// TODO: Add errors
    pub async fn build(self, signer: &impl Signer) -> Result<Read> {
        let descriptor = ReadDescriptor {
            base: Descriptor {
                interface: Interface::Records,
                method: Method::Read,
                message_timestamp: self.message_timestamp,
            },
            filter: self.filter.normalize()?,
        };

        let authorization = if self.authorize.unwrap_or(true) {
            let mut auth_builder =
                AuthorizationBuilder::new().descriptor_cid(cid::from_value(&descriptor)?);
            if let Some(id) = self.permission_grant_id {
                auth_builder = auth_builder.permission_grant_id(id);
            }
            if let Some(role) = self.protocol_role {
                auth_builder = auth_builder.protocol_role(role);
            }
            if let Some(delegated_grant) = self.delegated_grant {
                auth_builder = auth_builder.delegated_grant(delegated_grant);
            }
            Some(auth_builder.build(signer).await?)
        } else {
            None
        };

        Ok(Read {
            descriptor,
            authorization,
        })
    }
}
