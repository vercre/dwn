use jsonschema::error::ValidationError;
use jsonschema::{Retrieve, Uri};
use serde::Serialize;
use serde_json::Value;

use crate::endpoint::Message;
use crate::{unexpected, Result};

/// Validates the given payload using JSON schema keyed by the given schema name.
/// Throws if the given payload fails validation.
pub fn validate(message: &impl Message) -> Result<()> {
    let descriptor = message.descriptor();
    let schema_name = format!("{}-{}", descriptor.interface, descriptor.method).to_lowercase();
    validate_value(&schema_name, message)
}

/// Validates the given payload using JSON schema keyed by the given schema name.
/// Throws if the given payload fails validation.
pub fn validate_value<T: Serialize + ?Sized>(schema: &str, value: &T) -> Result<()> {
    let retriever = Retriever {};
    let schema = precompiled(schema)?;
    let validator = jsonschema::options().with_retriever(retriever).build(&schema)?;
    let instance = serde_json::to_value(value)?;

    // check for validation errors
    let errors: Vec<ValidationError> = validator.iter_errors(&instance).collect();
    if !errors.is_empty() {
        let mut error = String::new();
        for e in errors {
            error.push_str(&format!("\n - {e} at {}", e.instance_path));
        }
        return Err(unexpected!("validation failed for {schema}: {error}"));
    }

    Ok(())
}

/// Precompiled JSON schemas.
fn precompiled(schema_name: &str) -> Result<Value> {
    match schema_name {
        "messages-query" => {
            let schema = include_bytes!("../schemas/interface-methods/messages-query.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "messages-read" => {
            let schema = include_bytes!("../schemas/interface-methods/messages-read.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "messages-subscribe" => {
            let schema = include_bytes!("../schemas/interface-methods/messages-subscribe.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "protocols-configure" => {
            let schema = include_bytes!("../schemas/interface-methods/protocols-configure.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "protocols-query" => {
            let schema = include_bytes!("../schemas/interface-methods/protocols-query.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "records-write" => {
            let schema = include_bytes!("../schemas/interface-methods/records-write.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "records-read" => {
            let schema = include_bytes!("../schemas/interface-methods/records-read.json");
            Ok(serde_json::from_slice(schema)?)
        }
        "records-delete" => {
            let schema = include_bytes!("../schemas/interface-methods/records-delete.json");
            Ok(serde_json::from_slice(schema)?)
        }

        _ => Err(unexpected!("Schema not found: {schema_name}")),
    }
}

struct Retriever {
    // schemas: HashMap<String, Value>,
}

impl Retrieve for Retriever {
    fn retrieve(
        &self, uri: &Uri<&str>,
    ) -> Result<Value, Box<(dyn std::error::Error + Send + Sync + 'static)>> {
        let Some(file) = uri.path().split('/').last() else {
            return Err(unexpected!("Schema not found: {uri}").into());
        };

        match file.as_str() {
            "defs.json" => {
                let schema = include_bytes!("../schemas/definitions.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "messages-filter.json" => {
                let schema = include_bytes!("../schemas/interface-methods/messages-filter.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "pagination-cursor.json" => {
                let schema = include_bytes!("../schemas/interface-methods/pagination-cursor.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "protocol-definition.json" => {
                let schema =
                    include_bytes!("../schemas/interface-methods/protocol-definition.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "protocol-rule-set.json" => {
                let schema = include_bytes!("../schemas/interface-methods/protocol-rule-set.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "records-write-data-encoded.json" => {
                let schema =
                    include_bytes!("../schemas/interface-methods/records-write-data-encoded.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "records-write-unidentified.json" => {
                let schema =
                    include_bytes!("../schemas/interface-methods/records-write-unidentified.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "records-filter.json" => {
                let schema = include_bytes!("../schemas/interface-methods/records-filter.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "string-range-filter.json" => {
                let schema =
                    include_bytes!("../schemas/interface-methods/string-range-filter.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "number-range-filter.json" => {
                let schema =
                    include_bytes!("../schemas/interface-methods/number-range-filter.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "authorization.json" => {
                let schema = include_bytes!("../schemas/authorization.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "authorization-owner.json" => {
                let schema = include_bytes!("../schemas/authorization-owner.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "authorization-delegated-grant.json" => {
                let schema = include_bytes!("../schemas/authorization-delegated-grant.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "general-jws.json" => {
                let schema = include_bytes!("../schemas/general-jws.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "public-jwk.json" => {
                let schema = include_bytes!("../schemas/jwk/public-jwk.json");
                Ok(serde_json::from_slice(schema)?)
            }
            "general-jwk.json" => {
                let schema = include_bytes!("../schemas/jwk/general-jwk.json");
                Ok(serde_json::from_slice(schema)?)
            }

            _ => Err(unexpected!("Schema not found: {uri}").into()),
        }
    }
}
