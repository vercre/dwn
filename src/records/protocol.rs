use std::collections::BTreeMap;

use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::protocols::{self, Action, ActionRule, Actor, Definition, ProtocolType, RuleSet};
use crate::provider::MessageStore;
use crate::records::{self, Write};
use crate::service::Message;
use crate::{utils, Interface, Method};

/// Performs validation on the structure of `RecordsWrite` messages that use a protocol.
pub async fn verify_integrity(owner: &str, write: &Write, store: &impl MessageStore) -> Result<()> {
    let Some(protocol) = &write.descriptor.protocol else {
        return Err(anyhow!("missing protocol"));
    };
    let definition = protocol_definition(owner, protocol, store).await?;
    let Some(protocol_path) = &write.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol"));
    };
    let Some(rule_set) = rule_set(protocol_path, &definition.structure) else {
        return Err(anyhow!("no rule set defined for protocol path"));
    };

    verify_type(write, &definition.types)?;
    verify_protocol_path(owner, write, store).await?;
    if rule_set.role.is_some() {
        verify_role_record(owner, write, store).await?;
    }
    verify_size_limit(write.descriptor.data_size, &rule_set)?;
    verify_tags(write.descriptor.tags.as_ref(), &rule_set)?;

    Ok(())
}

/// Verifies the `data_format` and `schema` parameters .
fn verify_type(write: &Write, types: &BTreeMap<String, ProtocolType>) -> Result<()> {
    let Some(protocol_path) = &write.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol path"));
    };
    let Some(type_name) = protocol_path.split('/').last() else {
        return Err(anyhow!("missing type name"));
    };
    let Some(protocol_type) = types.get(type_name) else {
        return Err(anyhow!("record with type {type_name} not allowed in protocol"));
    };

    if protocol_type.schema.is_some() && protocol_type.schema != write.descriptor.schema {
        return Err(anyhow!("invalid schema for type {type_name}"));
    }

    if let Some(data_formats) = &protocol_type.data_formats {
        if !data_formats.contains(&write.descriptor.data_format) {
            return Err(anyhow!("invalid data_format for type {type_name}"));
        }
    }

    Ok(())
}

// Verify the `protocol_path` matches the path of actual record chain.
async fn verify_protocol_path(owner: &str, write: &Write, store: &impl MessageStore) -> Result<()> {
    let Some(protocol_path) = &write.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol path"));
    };
    let Some(type_name) = protocol_path.split('/').last() else {
        return Err(anyhow!("missing type name"));
    };

    // fetch the parent message
    let Some(parent_id) = &write.descriptor.parent_id else {
        if protocol_path != type_name {
            return Err(anyhow!("invalid protocol path for parentless record"));
        }
        return Ok(());
    };
    let Some(protocol) = &write.descriptor.protocol else {
        return Err(anyhow!("missing protocol"));
    };

    let sql = format!(
        "
        WHERE descriptor.interface = '{interface}'
        AND descriptor.method = '{method}'
        AND descriptor.protocol = '{protocol}'
        AND recordId = '{parent_id}'
        ",
        interface = Interface::Records,
        method = Method::Write,
        // AND isLatestBaseState = true
    );

    let (records, _) = store.query(owner, &sql).await?;
    if records.is_empty() {
        return Err(anyhow!("unable to find Write Record for parent_id {parent_id}"));
    }
    let Message::RecordsWrite(parent) = &records[0] else {
        return Err(anyhow!("unexpected message type"));
    };

    // verify protocol_path is a child of the parent message's protocol_path
    let Some(parent_path) = &parent.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol path"));
    };
    if &format!("{parent_path}/${type_name}") != protocol_path {
        return Err(anyhow!("invalid `protocol_path`"));
    }

    // verifying context_id is a child of the parent's context_id
    let Some(context_id) = &write.context_id else {
        return Err(anyhow!("missing context_id"));
    };
    let Some(parent_context_id) = &parent.context_id else {
        return Err(anyhow!("missing parent context_id"));
    };
    if context_id != &format!("{parent_context_id}/{}", write.record_id) {
        return Err(anyhow!("invalid `context_id`"));
    }

    Ok(())
}

/// Verify the integrity of the `records::Write` as a role record.
async fn verify_role_record(owner: &str, write: &Write, store: &impl MessageStore) -> Result<()> {
    let Some(recipient) = &write.descriptor.recipient else {
        return Err(anyhow!("role record is missing recipient"));
    };
    let Some(protocol) = &write.descriptor.protocol else {
        return Err(anyhow!("missing protocol"));
    };
    let Some(protocol_path) = &write.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol_path"));
    };

    // if this is not the root record, add a prefix filter to the query
    let mut context = String::new();
    if let Some(parent_context) =
        write.context_id.as_ref().and_then(|context_id| context_id.rsplit_once('/').map(|x| x.0))
    {
        context =
            format!("AND contextId BETWEEN '{parent_context}' AND '{parent_context}\u{ffff}'");
    };

    let sql = format!(
        "
        WHERE descriptor.interface = '{interface}'
        AND descriptor.method = '{method}'
        AND descriptor.protocol = '{protocol}'
        AND descriptor.protocolPath = '{protocol_path}'
        AND recipient = '{recipient}'
        {context}
        ",
        interface = Interface::Records,
        method = Method::Write,
        // AND isLatestBaseState = true
    );

    let (records, _) = store.query(owner, &sql).await?;
    // if records.is_empty() {
    //     return Err(anyhow!("unable to find Write Record for parent_id {parent_id}"));
    // }

    for record in records {
        if let Message::RecordsWrite(matched) = record {
            if matched.record_id != write.record_id {
                return Err(anyhow!("DID '{recipient}' is already recipient of a role record"));
            }
        }
    }

    Ok(())
}

// Check if the incoming message is invoking a role. If so, validate the invoked role.
async fn verify_invoked_role(
    owner: &str, write: &Write, definition: &Definition, store: &impl MessageStore,
) -> Result<()> {
    let Some(protocol_role) = write.authorization.jws_payload()?.protocol_role else {
        return Ok(());
    };
    let author = write.authorization.author()?;

    let Some(protocol_uri) = &write.descriptor.protocol else {
        return Err(anyhow!("missing protocol"));
    };

    let Some(rule_set) = rule_set(&protocol_role, &definition.structure) else {
        return Err(anyhow!("no rule set defined for protocol role"));
    };
    if !rule_set.role.unwrap_or_default() {
        return Err(anyhow!("protocol path {protocol_role} does not match role record type"));
    }

    let segment_count = protocol_role.split('/').count();
    if write.context_id.is_none() && segment_count > 1 {
        return Err(anyhow!("unable verify role without `context_id`"));
    }

    // `context_id` prefix filter
    let context_prefix = if segment_count > 0 {
        // context_id segment count is never shorter than the role path count.
        let context_id = write.context_id.clone().unwrap_or_default();
        let context_id_segments: Vec<&str> = context_id.split('/').collect();
        let prefix = context_id_segments[..segment_count].join("/");
        format!("AND contextId BETWEEN '{prefix}' AND '{prefix}\u{ffff}'")
    } else {
        String::new()
    };

    // fetch the invoked role record
    let sql = format!(
        "
        WHERE descriptor.interface = '{interface}'
        AND descriptor.method = '{method}'
        AND protocol = '{protocol_uri}'
        AND protocolPath = '{protocol_role}'
        AND recipient = '{author}'
        {context_prefix}
        ",
        interface = Interface::Records,
        method = Method::Write,
        // isLatestBaseState : true,
    );
    let (records, _) = store.query(owner, &sql).await?;
    if records.is_empty() {
        return Err(anyhow!("unable to find records for {protocol_role}"));
    }

    Ok(())
}

// Verify write record adheres to the $size constraints.
fn verify_size_limit(data_size: u64, rule_set: &RuleSet) -> Result<()> {
    let Some(range) = &rule_set.size else {
        return Ok(());
    };

    if let Some(min) = range.min {
        if data_size < min {
            return Err(anyhow!("data size is less than allowed"));
        }
    }
    if let Some(max) = range.max {
        if data_size > max {
            return Err(anyhow!("data size is greater than allowed"));
        }
    }

    Ok(())
}

fn verify_tags(tags: Option<&Map<String, Value>>, rule_set: &RuleSet) -> Result<()> {
    let Some(rule_set_tags) = &rule_set.tags else {
        return Ok(());
    };

    let additional_properties = rule_set_tags.allow_undefined_tags.unwrap_or_default();
    let required_default = vec![];
    let required = rule_set_tags.required_tags.as_ref().unwrap_or(&required_default);
    let properties = &rule_set_tags.undefined_tags;

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": additional_properties,
    });

    // validate tags against schema
    let instance = serde_json::to_value(tags)?;
    if !jsonschema::is_valid(&schema, &instance) {
        return Err(anyhow!("tags do not match schema"));
    }

    Ok(())
}

// Verifies the given message is authorized by one of the action rules in the given protocol rule set.
async fn verify_actions(
    owner: &str, write: &Write, rule_set: &RuleSet, record_chain: Vec<Write>,
    store: &impl MessageStore,
) -> Result<()> {
    let author = write.authorization.author()?;
    let allowed_actions = allowed_actions(owner, write, store).await?;

    // NOTE: We have already checked that the message is not from tenant, owner,
    // or permission grant authorized prior to this method being called.

    let Some(action_rules) = &rule_set.actions else {
        return Err(anyhow!("no action rule defined for RecordsWrite, ${author} is unauthorized"));
    };

    let invoked_role = write.authorization.jws_payload()?.protocol_role;

    // find a rule that authorizes the incoming message
    for rule in action_rules {
        if !rule.can.iter().any(|action| allowed_actions.contains(action)) {
            continue;
        }
        if rule.who == Some(Actor::Anyone) {
            return Ok(());
        }
        // if author.is_none() {
        //     continue;
        // }

        // role validation
        if invoked_role.is_some() {
            if rule.role == invoked_role {
                return Ok(());
            }
            continue;
        }

        // actor validation
        if rule.who == Some(Actor::Recipient) && rule.of.is_none() {
            let message = if write.descriptor.base.method == Method::Write {
                write
            } else {
                // the incoming message must be a `RecordsDelete` because only
                // `co-update`, `co-delete`, `co-prune` are allowed recipient actions,
                &record_chain[record_chain.len() - 1]
            };

            if message.descriptor.recipient.as_ref() == Some(&author) {
                return Ok(());
            }
            continue;
        }

        // is actor allowed by the current action rule?
        if check_actor(&author, rule, &record_chain)? {
            return Ok(());
        }
    }

    Err(anyhow!("RecordsWrite by {author} not allowed"))
}

// Protocol-based authorization for records::Write messages.
pub async fn permit_write(owner: &str, write: &Write, store: &impl MessageStore) -> Result<()> {
    let record_id = &write.record_id;

    let record_chain = if records::initial_entry(owner, record_id, store).await?.is_some() {
        record_chain(owner, &write.record_id, store).await?
    } else {
        // NOTE: we can assume this message is an initial write because an existing
        // initial write does not exist.  Additionally, we check further down in the
        // `RecordsWriteHandler` if the incoming message is an initialWrite,
        // so we don't check explicitly here to avoid an unnecessary duplicate check.
        if let Some(parent_id) = &write.descriptor.parent_id {
            record_chain(owner, parent_id, store).await?
        } else {
            vec![]
        }
    };

    // protocol definition
    let Some(protocol) = &write.descriptor.protocol else {
        return Err(anyhow!("missing protocol"));
    };
    let definition = protocol_definition(owner, protocol, store).await?;

    // rule set
    let Some(protocol_path) = &write.descriptor.protocol_path else {
        return Err(anyhow!("missing protocol"));
    };
    let Some(rule_set) = rule_set(protocol_path, &definition.structure) else {
        return Err(anyhow!("no rule set defined for protocol path"));
    };

    verify_invoked_role(owner, write, &definition, store).await?;
    verify_actions(owner, write, &rule_set, record_chain, store).await?;

    Ok(())
}

// Fetches the protocol definition for the the protocol specified in the message.
async fn protocol_definition(
    owner: &str, protocol_uri: &str, store: &impl MessageStore,
) -> Result<Definition> {
    let protocol_uri = utils::clean_url(protocol_uri)?;

    // use default definition if first-class protocol
    if protocol_uri == protocols::PROTOCOL_URI {
        return Ok(Definition::default());
    }

    // fetch the corresponding protocol definition
    let sql = format!(
        "
        WHERE descriptor.interface = '{interface}'
        AND descriptor.method = '{method}'
        AND descriptor.definition.protocol = '{protocol_uri}'
        ",
        interface = Interface::Protocols,
        method = Method::Configure,
    );

    let (protocols, _) = store.query(owner, &sql).await?;
    if protocols.is_empty() {
        return Err(anyhow!("unable to find protocol definition for {protocol_uri}"));
    }
    let Message::ProtocolsConfigure(protocol_message) = &protocols[0] else {
        return Err(anyhow!("unexpected message type"));
    };

    Ok(protocol_message.descriptor.definition.clone())
}

fn rule_set(protocol_path: &str, structure: &BTreeMap<String, RuleSet>) -> Option<RuleSet> {
    let Some((type_name, protocol_path)) = protocol_path.split_once('/') else {
        return structure.get(protocol_path).cloned();
    };
    rule_set(protocol_path, &structure.get(type_name)?.structure)
}

// Constructs the chain of EXISTING records in the datastore where the first
// record is the root initial `records::Write` of the record chain and last
// record is the initial `records::Write` of the descendant record specified.
async fn record_chain(
    owner: &str, record_id: &str, store: &impl MessageStore,
) -> Result<Vec<Write>> {
    let mut chain = vec![];

    // keep walking up the chain from the inbound message's parent, until there
    // is no more parent
    let mut current_id = Some(record_id.to_owned());

    while let Some(record_id) = &current_id {
        let Some(initial_entry) = records::initial_entry(owner, record_id, store).await? else {
            return Err(anyhow!(
                "no parent found with ID {record_id} when constructing record chain."
            ));
        };

        chain.push(initial_entry.clone());
        current_id.clone_from(&initial_entry.descriptor.parent_id);
    }

    // root record first
    chain.reverse();
    Ok(chain)
}

// Match `Action`s that authorize the incoming message.
//
// N.B. keep in mind an author's 'write' access may be revoked.
async fn allowed_actions(
    owner: &str, write: &Write, store: &impl MessageStore,
) -> Result<Vec<Action>> {
    match write.descriptor.base.method {
        Method::Write => {
            if write.is_initial()? {
                return Ok(vec![Action::Create]);
            }
            let Some(initial_entry) =
                records::initial_entry(owner, &write.record_id, store).await?
            else {
                return Ok(Vec::new());
            };
            if write.authorization.author()? == initial_entry.authorization.author()? {
                return Ok(vec![Action::CoUpdate, Action::Update]);
            }

            Ok(vec![Action::CoUpdate])
        }
        Method::Query => Ok(vec![Action::Query]),
        Method::Read => Ok(vec![Action::Read]),
        Method::Subscribe => Ok(vec![Action::Subscribe]),
        Method::Delete => {
            //   let Message::RecordsDelete(delete) = message else {
            //     return Err(anyhow!("unexpected message type"));
            //   };
            //   let Some(initial_entry) = records::initial_entry(owner, &delete.record_id, store).await? else {
            //     return Ok(vec![]);
            //   };

            let actions = vec![];
            // let author= delete.authorization.author()?;
            // let initial_author = initial_entry.authorization.author()?;

            //   if delete.descriptor.prune {
            //     actions.push(Action::CoPrune);
            //     if author == initial_author {
            //       actions.push(Action::Prune);
            //     }
            //   }

            //   actions.push(Action::CoDelete);
            //   if author === initial_author {
            //       actions.push(Action::Delete);
            //   }

            Ok(actions)
        }

        Method::Configure => Err(anyhow!("configure method not allowed")),
    }
}

// Checks for a match with the `who` rule in record chain.
fn check_actor(author: &str, action_rule: &ActionRule, record_chain: &[Write]) -> Result<bool> {
    // find a message with matching protocolPath
    let ancestor =
        record_chain.iter().find(|write| write.descriptor.protocol_path == action_rule.of);
    let Some(ancestor) = ancestor else {
        // reaching this block means there is an issue with the protocol definition
        // this check should happen `protocols::Configure`
        return Ok(false);
    };
    if action_rule.who == Some(Actor::Recipient) {
        return Ok(Some(author.to_owned()) == ancestor.descriptor.recipient);
    }
    Ok(author == ancestor.authorization.author()?)
}