//! # Service
//!
//! Decentralized Web Node messaging framework.

use std::fmt::Debug;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::auth::Authorization;
use crate::provider::Provider;
use crate::{Descriptor, Result, schema};

/// Handle incoming messages.
///
/// # Errors
/// TODO: Add errors
pub async fn handle<T>(
    owner: &str, message: impl Message<Reply = T>, provider: &impl Provider,
) -> Result<Reply<T>> {
    message.validate().await?;
    message.handle(owner, provider).await
}

/// Methods common to all messages.
#[async_trait]
pub trait Message: Serialize + Clone + Debug + Send + Sync {
    /// The message's inner reply type.
    type Reply;

    /// Compute the CID of the message.
    ///
    /// # Errors
    /// TODO: Add errors
    fn cid(&self) -> Result<String>;

    /// Returns the component of the message descriptor common to all messages
    fn descriptor(&self) -> &Descriptor;

    /// Returns the messages's authorization, if set.
    fn authorization(&self) -> Option<&Authorization>;

    /// Handle the message.
    async fn handle(self, owner: &str, provider: &impl Provider) -> Result<Reply<Self::Reply>>;

    /// Validate the message. This is a generic validation common to all messages.
    /// Message-specific validation is done in the message handler.
    async fn validate(&self) -> Result<()> {
        // if !tenant_gate.active(owner)? {
        //     return Err(Error::Unauthorized("tenant not active"));
        // }
        schema::validate(self)
    }
}

/// Reply used by all endpoints.
#[derive(Debug, Default, Deserialize, Serialize)]
#[allow(clippy::module_name_repetitions)]
pub struct Reply<T> {
    /// Status message to accompany the reply.
    pub status: Status,

    /// Endpoint-specific reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(flatten)]
    pub body: Option<T>,
}

/// Reply status.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Status code.
    pub code: u16,

    /// Status detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
