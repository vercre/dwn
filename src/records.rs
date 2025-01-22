//! # Records

mod delete;
mod encryption;
mod query;
mod read;
mod subscribe;
pub(crate) mod write;

use std::collections::BTreeMap;
use std::fmt::Display;

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use self::delete::{Delete, DeleteDescriptor};
pub use self::encryption::{EncryptOptions, EncryptedKey, EncryptionProperty, Recipient, decrypt};
pub use self::query::{Query, QueryDescriptor};
pub use self::read::{Read, ReadDescriptor};
pub use self::subscribe::{Subscribe, SubscribeDescriptor, SubscribeReply};
pub use self::write::{Attestation, DelegatedGrant, SignaturePayload, Write, WriteDescriptor};
pub use crate::data::DataStream;
use crate::{DateRange, Lower, OneOrMany, Range, Result, Upper, utils};

// TODO: add builder for RecordsFilter

/// Records filter.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordsFilter {
    /// Get a single object by its ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,

    /// Records matching the specified author.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<OneOrMany<String>>,

    /// Records matching the specified creator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attester: Option<String>,

    /// Records matching the specified recipient(s).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient: Option<OneOrMany<String>>,

    /// Records with the specified context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,

    /// The CID of the parent object .
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,

    /// Entry matching the specified protocol.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,

    /// Entry protocol path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_path: Option<String>,

    /// Records with the specified schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// The MIME type of the requested data. For example, `application/json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_format: Option<String>,

    /// Match records with the specified tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<BTreeMap<String, TagFilter>>,

    /// CID of the data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_cid: Option<String>,

    /// Records with a size within the range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_size: Option<Range<usize>>,

    /// Whether the record is published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<bool>,

    /// Filter messages published within the specified range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_published: Option<DateRange>,

    /// Filter messages created within the specified range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_created: Option<DateRange>,

    /// Match messages updated within the specified range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_updated: Option<DateRange>,
}

/// Filter value.
pub enum FilterVal {
    /// Filter on an exact value.
    Equal(String),

    /// Filter on one or more values.
    OneOf(Vec<String>),

    /// Filter on a date range.
    Range(Range<String>),
    //
    // /// Filter on entries starting with specified value.
    // StartsWith(String),
}

impl RecordsFilter {
    /// Normalizes `RecordsFilter` protocol and schema URLs within a provided.
    pub(crate) fn normalize(&self) -> Result<Self> {
        let mut filter = self.clone();
        filter.protocol = if let Some(protocol) = &self.protocol {
            Some(utils::clean_url(protocol)?)
        } else {
            None
        };
        filter.schema =
            if let Some(schema) = &self.schema { Some(utils::clean_url(schema)?) } else { None };

        Ok(filter)
    }

    /// Create an optimized filter to use with single-field indexes. This
    /// method chooses the best filter property, in order of priority, to use
    /// when querying.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn to_concise(&self) -> Option<(String, FilterVal)> {
        if let Some(record_id) = &self.record_id {
            return Some(("record_id".to_string(), FilterVal::Equal(record_id.clone())));
        }
        if let Some(attester) = &self.attester {
            return Some(("attester".to_string(), FilterVal::Equal(attester.clone())));
        }
        if let Some(parent_id) = &self.parent_id {
            return Some(("parent_id".to_string(), FilterVal::Equal(parent_id.clone())));
        }
        if let Some(recipient) = &self.recipient {
            let recipients = match recipient {
                OneOrMany::One(recipient) => vec![recipient.clone()],
                OneOrMany::Many(recipients) => recipients.clone(),
            };
            return Some(("recipient".to_string(), FilterVal::OneOf(recipients)));
        }
        if let Some(context_id) = &self.context_id {
            return Some(("context_id".to_string(), FilterVal::Equal(context_id.clone())));
        }
        if let Some(protocol_path) = &self.protocol_path {
            return Some(("protocol_path".to_string(), FilterVal::Equal(protocol_path.clone())));
        }
        if let Some(schema) = &self.schema {
            return Some(("schema".to_string(), FilterVal::Equal(schema.clone())));
        }
        if let Some(protocol) = &self.protocol {
            return Some(("protocol".to_string(), FilterVal::Equal(protocol.clone())));
        }
        if let Some(data_cid) = &self.data_cid {
            return Some(("data_cid".to_string(), FilterVal::Equal(data_cid.clone())));
        }
        if let Some(data_size) = &self.data_size {
            let lower = data_size.lower.as_ref().map(|lower| match lower {
                Lower::Inclusive(val) => Lower::Inclusive(format!("{val:0>10}")),
                Lower::Exclusive(val) => Lower::Exclusive(format!("{val:0>10}")),
            });
            let upper = data_size.upper.as_ref().map(|upper| match upper {
                Upper::Inclusive(val) => Upper::Inclusive(format!("{val:0>10}")),
                Upper::Exclusive(val) => Upper::Exclusive(format!("{val:0>10}")),
            });
            return Some(("data_size".to_string(), FilterVal::Range(Range { lower, upper })));
        }

        // TODO: move DateRange -> Range<String> conversion to a separate method
        if let Some(date_published) = &self.date_published {
            let mut range = Range::default();
            if let Some(lower) = &date_published.lower {
                let lower = lower.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.lower = Some(Lower::Inclusive(lower));
            }
            if let Some(upper) = &date_published.upper {
                let upper = upper.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.upper = Some(Upper::Inclusive(upper));
            }
            return Some(("date_published".to_string(), FilterVal::Range(range)));
        }
        if let Some(date_created) = &self.date_created {
            let mut range = Range::default();
            if let Some(lower) = &date_created.lower {
                let lower = lower.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.lower = Some(Lower::Inclusive(lower));
            }
            if let Some(upper) = &date_created.upper {
                let upper = upper.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.upper = Some(Upper::Inclusive(upper));
            }
            return Some(("date_created".to_string(), FilterVal::Range(range)));
        }
        if let Some(date_updated) = &self.date_updated {
            let mut range = Range::default();
            if let Some(lower) = &date_updated.lower {
                let lower = lower.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.lower = Some(Lower::Inclusive(lower));
            }
            if let Some(upper) = &date_updated.upper {
                let upper = upper.to_rfc3339_opts(SecondsFormat::Micros, true);
                range.upper = Some(Upper::Inclusive(upper));
            }
            return Some(("date_updated".to_string(), FilterVal::Range(range)));
        }

        if let Some(data_format) = &self.data_format {
            return Some(("data_format".to_string(), FilterVal::Equal(data_format.clone())));
        }
        if let Some(published) = self.published {
            return Some(("published".to_string(), FilterVal::Equal(published.to_string())));
        }
        if let Some(author) = &self.author {
            let authors = match author {
                OneOrMany::One(author) => vec![author.to_string()],
                OneOrMany::Many(authors) => authors.clone(),
            };
            return Some(("author".to_string(), FilterVal::OneOf(authors)));
        }

        // TODO: improve this logic
        if let Some(tags) = &self.tags {
            if let Some((key, filter)) = tags.iter().next() {
                let tag_key = format!("tag.{key}");

                match filter {
                    TagFilter::Equal(value) => {
                        return Some((tag_key, FilterVal::Equal(value.to_string())));
                    }
                    TagFilter::Range(range) => {
                        let lower = range.lower.as_ref().map(|lower| match lower {
                            Lower::Inclusive(val) => Lower::Inclusive(format!("{val:0>10}")),
                            Lower::Exclusive(val) => Lower::Exclusive(format!("{val:0>10}")),
                        });
                        let upper = range.upper.as_ref().map(|upper| match upper {
                            Upper::Inclusive(val) => Upper::Inclusive(format!("{val:0>10}")),
                            Upper::Exclusive(val) => Upper::Exclusive(format!("{val:0>10}")),
                        });
                        return Some((tag_key, FilterVal::Range(Range { lower, upper })));
                    }
                    TagFilter::StartsWith(value) => {
                        return Some((tag_key, FilterVal::Equal(value.to_string())));
                    }
                }
            }
        }

        None
    }
}

/// `EntryType` sort.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Sort {
    /// Sort `date_created` from oldest to newest.
    #[serde(rename="createdAscending")]
    CreatedAsc,

    /// Sort `date_created` newest to oldest.
    #[serde(rename="createdDescending")]
    CreatedDesc,

    /// Sort `date_published` from oldest to newest.
    #[serde(rename="publishedAscending")]
    PublishedAsc,

    /// Sort `date_published` from newest to oldest.
    #[serde(rename="publishedDescending")]
    PublishedDesc,

    /// Sort `message_timestamp` from oldest to newest.
    #[serde(rename="timestampAscending")]
    #[default]
    TimestampAsc,

    /// Sort `message_timestamp` from newest to oldest.
    #[serde(rename="timestampDescending")]
    TimestampDesc,
}

impl Display for Sort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreatedAsc | Self::CreatedDesc => write!(f, "dateCreated"),
            Self::PublishedAsc | Self::PublishedDesc => write!(f, "datePublished"),
            Self::TimestampAsc | Self::TimestampDesc => write!(f, "messageTimestamp"),
        }
    }
}

/// Tag filter.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TagFilter {
    /// Match tags starting with a string value.
    StartsWith(String),

    /// Filter tags by range.
    Range(Range<usize>),

    /// Filter by a specific value.
    Equal(Value),
}

impl Default for TagFilter {
    fn default() -> Self {
        Self::Equal(Value::Null)
    }
}

/// Implement  builder-like behaviour.
impl RecordsFilter {
    /// Returns a new [`RecordsFilter`]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one or more authors to the filter.
    #[must_use]
    pub fn add_author(mut self, author: impl Into<String>) -> Self {
        match &mut self.author {
            Some(OneOrMany::Many(existing)) => {
                existing.push(author.into());
            }
            Some(OneOrMany::One(existing)) => {
                self.author = Some(OneOrMany::Many(vec![existing.clone(), author.into()]));
            }
            None => {
                self.author = Some(OneOrMany::One(author.into()));
            }
        }
        self
    }

    /// Add an attester to the filter.
    #[must_use]
    pub fn attester(mut self, attester: impl Into<String>) -> Self {
        self.attester = Some(attester.into());
        self
    }

    /// Add one or more recipients to the filter.
    #[must_use]
    pub fn add_recipient(mut self, recipient: impl Into<String>) -> Self {
        match &mut self.recipient {
            Some(OneOrMany::Many(existing)) => {
                existing.push(recipient.into());
            }
            Some(OneOrMany::One(existing)) => {
                self.recipient = Some(OneOrMany::Many(vec![existing.clone(), recipient.into()]));
            }
            None => {
                self.recipient = Some(OneOrMany::One(recipient.into()));
            }
        }
        self
    }

    /// Add a protocol to the filter.
    #[must_use]
    pub fn protocol(mut self, protocol: impl Into<String>) -> Self {
        self.protocol = Some(protocol.into());
        self
    }

    /// Add a protocol path to the filter.
    #[must_use]
    pub fn protocol_path(mut self, protocol_path: impl Into<String>) -> Self {
        self.protocol_path = Some(protocol_path.into());
        self
    }

    /// Specify a protocol schema on the filter.
    #[must_use]
    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    /// Add a published flag to the filter.
    #[must_use]
    pub const fn published(mut self, published: bool) -> Self {
        self.published = Some(published);
        self
    }

    /// Add a context ID to the filter.
    #[must_use]
    pub fn context_id(mut self, context_id: impl Into<String>) -> Self {
        self.context_id = Some(context_id.into());
        self
    }

    /// Add a record ID to the filter.
    #[must_use]
    pub fn record_id(mut self, record_id: impl Into<String>) -> Self {
        self.record_id = Some(record_id.into());
        self
    }

    /// Add a parent ID to the filter.
    #[must_use]
    pub fn parent_id(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Add a tag to the filter.
    #[must_use]
    pub fn add_tag(mut self, key: impl Into<String>, value: TagFilter) -> Self {
        if let Some(existing) = &mut self.tags {
            existing.insert(key.into(), value);
        } else {
            let mut tags = BTreeMap::new();
            tags.insert(key.into(), value);
            self.tags = Some(tags);
        }
        self
    }

    /// Add a data format to the filter.
    #[must_use]
    pub fn data_format(mut self, data_format: impl Into<String>) -> Self {
        self.data_format = Some(data_format.into());
        self
    }

    /// Add a data size to the filter.
    #[must_use]
    pub const fn data_size(mut self, data_size: Range<usize>) -> Self {
        self.data_size = Some(data_size);
        self
    }

    /// Add a data CID to the filter.
    #[must_use]
    pub fn data_cid(mut self, data_cid: impl Into<String>) -> Self {
        self.data_cid = Some(data_cid.into());
        self
    }

    /// Add a date created to the filter.
    #[must_use]
    pub const fn date_created(mut self, date_created: DateRange) -> Self {
        self.date_created = Some(date_created);
        self
    }

    /// Add a date published to the filter.
    #[must_use]
    pub const fn date_published(mut self, date_published: DateRange) -> Self {
        self.date_published = Some(date_published);
        self
    }

    /// Add a date updated to the filter.
    #[must_use]
    pub const fn date_updated(mut self, date_updated: DateRange) -> Self {
        self.date_updated = Some(date_updated);
        self
    }
}
