//! Records Read

use std::io::Read;

use base64ct::{Base64UrlUnpadded, Encoding};
use dwn_test::key_store::{ALICE_DID, ALICE_VERIFYING_KEY, BOB_DID, BOB_VERIFYING_KEY, CAROL_DID};
use dwn_test::provider::ProviderImpl;
use http::StatusCode;
use rand::RngCore;
use serde_json::Value;
use vercre_dwn::data::{DataStream, MAX_ENCODED_SIZE};
use vercre_dwn::hd_key::{
    self, DerivationPath, DerivationScheme, DerivedPrivateJwk, PrivateKeyJwk,
};
use vercre_dwn::permissions::{GrantBuilder, RecordsScope, Scope};
use vercre_dwn::protocols::{ConfigureBuilder, Definition, QueryBuilder};
use vercre_dwn::provider::{BlockStore, KeyStore, MessageStore};
use vercre_dwn::records::{
    Data, DeleteBuilder, EncryptOptions, ProtocolSettings, ReadBuilder, Recipient, RecordsFilter,
    WriteBuilder, decrypt,
};
use vercre_dwn::store::Entry;
use vercre_dwn::{Error, Method, endpoint};
use vercre_infosec::jose::{Curve, KeyType, PublicKeyJwk};

// Should allow an owner to read their own records.
#[tokio::test]
async fn owner() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Add a `write` record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Read the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let record = body.entry.records_write.expect("should have records_write");
    assert_eq!(record.record_id, write.record_id);
}

// Should not allow non-owners to read private records.
#[tokio::test]
async fn disallow_non_owner() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record but fails.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "read cannot be authorized");
}

// Should allow anonymous users to read published records.
#[tokio::test]
async fn published_anonymous() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Add a `write` record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Read the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .build()
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
}

// Should allow authenticated users to read published records.
#[tokio::test]
async fn published_authenticated() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
}

// Should allow non-owners to read records they have received.
#[tokio::test]
async fn non_owner_recipient() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .recipient(BOB_DID)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
}

// Should return BadRequest (400) when attempting to fetch a deleted record
// using a valid `record_id`.
#[tokio::test]
async fn deleted_write() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Mock write and delete, saving only the `RecordsDelete`.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .recipient(BOB_DID)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    let delete = DeleteBuilder::new()
        .record_id(&write.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");

    let mut initial = Entry::from(&write);
    initial.indexes.insert("recordId".to_string(), Value::String(write.record_id.clone()));
    let mut entry = Entry::from(&delete);
    entry.indexes.extend(initial.indexes);

    MessageStore::put(&provider, ALICE_DID, &entry).await.expect("should save");

    // --------------------------------------------------
    // Alice attempts to read the record and gets an error.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "initial write for deleted record not found");
}

// Should return Forbidden (403) when non-authors attempt to fetch the initial
// write of a deleted record using a valid `record_id`.
#[tokio::test]
async fn non_author_deleted_write() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures a protocol allowing anyone to write.
    // --------------------------------------------------
    let def_json = serde_json::json!({
        "published" : true,
        "protocol"  : "https://example.com/foo",
        "types"     : {
            "foo": {}
        },
        "structure": {
            "foo": {
                "$actions": [{
                    "who" : "anyone",
                    "can" : ["create", "delete"]
                }]
            }
        }
    });
    let definition: Definition = serde_json::from_value(def_json).expect("should deserialize");

    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes a record to Alice's web node.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "https://example.com/foo".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob deletes the record.
    // --------------------------------------------------
    let delete = DeleteBuilder::new()
        .record_id(&write.record_id)
        .build(&bob_keyring)
        .await
        .expect("should create delete");
    let reply = endpoint::handle(ALICE_DID, delete, &provider).await.expect("should read");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Carol attempts to read the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should allow non-owners to read records they have authored.
#[tokio::test]
async fn non_owner_author() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures a protocol allowing anyone to write.
    // --------------------------------------------------
    let def_json = serde_json::json!({
        "published" : true,
        "protocol"  : "https://example.com/foo",
        "types"     : {
            "foo": {}
        },
        "structure": {
            "foo": {
                "$actions": [{
                    "who" : "anyone",
                    "can" : ["create"]
                }]
            }
        }
    });
    let definition: Definition = serde_json::from_value(def_json).expect("should deserialize");

    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes a record to Alice's web node.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "https://example.com/foo".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads his record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());

    // --------------------------------------------------
    // Carol attempts to read the record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should include intial write for updated records.
#[tokio::test]
async fn initial_write_included() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record and then an update.
    // --------------------------------------------------
    let write_1 = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let write_2 = WriteBuilder::from(write_1)
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice reads her record which includes the `initial_write`.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write_2.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.initial_write.is_some());
}

// Should allow anyone to read when using `allow-anyone` rule.
#[tokio::test]
async fn allow_anyone() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a social media protocol.
    // --------------------------------------------------
    let social_media = include_bytes!("../crates/dwn-test/protocols/social-media.json");
    let definition: Definition = serde_json::from_slice(social_media).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice saves an image.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"cafe-aesthetic.jpg".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://social-media.xyz".to_string(),
            protocol_path: "image".to_string(),
            parent_context_id: None,
        })
        .schema("imageSchema")
        .data_format("image/jpeg")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads the image.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
}

// Should not allow anonymous reads when there is no `allow-anyone` rule.
#[tokio::test]
async fn no_anonymous() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures an email protocol.
    // --------------------------------------------------
    let email = include_bytes!("../crates/dwn-test/protocols/email.json");
    let definition: Definition = serde_json::from_slice(email).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes an email.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"foo".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
            parent_context_id: None,
        })
        .schema("email")
        .data_format("text/plain")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // An anonymous users attempts to read the message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .build()
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "read not authorized");
}

// Should allow read using recipient rule.
#[tokio::test]
async fn allow_recipient() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures an email protocol.
    // --------------------------------------------------
    let email = include_bytes!("../crates/dwn-test/protocols/email.json");
    let definition: Definition = serde_json::from_slice(email).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes an email to Bob.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"Hello Bob!".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
            parent_context_id: None,
        })
        .schema("email")
        .data_format("text/plain")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads the email.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());

    // --------------------------------------------------
    // Carol attempts to read the email.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should allow read using ancestor author rule.
#[tokio::test]
async fn allow_author() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures an email protocol.
    // --------------------------------------------------
    let email = include_bytes!("../crates/dwn-test/protocols/email.json");
    let definition: Definition = serde_json::from_slice(email).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes an email to Alice.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"Hello Alice!".to_vec()))
        .recipient(ALICE_DID)
        .protocol(ProtocolSettings {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
            parent_context_id: None,
        })
        .schema("email")
        .data_format("text/plain")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads his email.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());

    // --------------------------------------------------
    // Carol attempts to read the email.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should support using a filter when there is only a single result.
#[tokio::test]
async fn filter_one() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures a nested protocol.
    // --------------------------------------------------
    let nested = include_bytes!("../crates/dwn-test/protocols/nested.json");
    let definition: Definition = serde_json::from_slice(nested).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a message to a nested protocol.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"foo".to_vec()))
        .recipient(ALICE_DID)
        .protocol(ProtocolSettings {
            protocol: "http://nested.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .schema("foo")
        .data_format("text/plain")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice reads the message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().protocol("http://nested.xyz").protocol_path("foo"))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
}

// Should return a status of BadRequest (400) when using a filter returns multiple results.
#[tokio::test]
async fn filter_many() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures a nested protocol.
    // --------------------------------------------------
    let nested = include_bytes!("../crates/dwn-test/protocols/nested.json");
    let definition: Definition = serde_json::from_slice(nested).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes 2 messages to a nested protocol.
    // --------------------------------------------------
    for _ in 0..2 {
        let write = WriteBuilder::new()
            .data(Data::from(b"foo".to_vec()))
            .recipient(ALICE_DID)
            .protocol(ProtocolSettings {
                protocol: "http://nested.xyz".to_string(),
                protocol_path: "foo".to_string(),
                parent_context_id: None,
            })
            .schema("foo")
            .data_format("text/plain")
            .sign(&alice_keyring)
            .build()
            .await
            .expect("should create write");
        let reply =
            endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
        assert_eq!(reply.status.code, StatusCode::ACCEPTED);
    }

    // --------------------------------------------------
    // Alice attempts to read one of the messages.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().protocol("http://nested.xyz").protocol_path("foo"))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "multiple messages exist");
}

// Should allow using a root-level role to authorize reads.
#[tokio::test]
async fn root_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a friend protocol.
    // --------------------------------------------------
    let friend = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes 2 messages to the protocol.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is a friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let chat = WriteBuilder::new()
        .data(Data::from(b"Bob can read this because he is a friend".to_vec()))
        .recipient(ALICE_DID)
        .protocol(ProtocolSettings {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads Alice's chat message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(chat.record_id))
        .protocol_role("friend")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should not allow reads when protocol path does not point to an active role record.
#[tokio::test]
async fn invalid_protocol_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a friend protocol.
    // --------------------------------------------------
    let friend = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a chat message to the protocol.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Blah blah blah".to_vec()))
        .recipient(ALICE_DID)
        .protocol(ProtocolSettings {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read Alice's chat message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(chat.record_id))
        .protocol_role("chat")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "protocol path does not match role record type");
}

// Should not allow reads when recipient does not have an active role.
#[tokio::test]
async fn no_recipient_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a friend protocol.
    // --------------------------------------------------
    let friend = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a chat message to the protocol.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Blah blah blah".to_vec()))
        .recipient(ALICE_DID)
        .protocol(ProtocolSettings {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read Alice's chat message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(chat.record_id))
        .protocol_role("friend")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "unable to find record for role");
}

// Should allow reads when using a valid context role.
#[tokio::test]
async fn context_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a thread protocol.
    // --------------------------------------------------
    let thread = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"A new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a participant on the thread.
    // --------------------------------------------------
    let participant = WriteBuilder::new()
        .data(Data::from(b"Bob is a friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
            parent_context_id: thread.context_id.clone(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, participant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a chat message on the thread.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Bob can read this because he is a participant".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/chat".to_string(),
            parent_context_id: thread.context_id.clone(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads his participant role record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(
            RecordsFilter::new()
                .protocol_path("thread/participant")
                .add_recipient(BOB_DID)
                .context_id(thread.context_id.as_ref().unwrap()),
        )
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);

    // --------------------------------------------------
    // Bob reads the thread root record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(participant.descriptor.parent_id.as_ref().unwrap()))
        .protocol_role("thread/participant")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);

    // --------------------------------------------------
    // Bob uses his participant role to read the chat message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(chat.record_id))
        .protocol_role("thread/participant")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should not allow reads when context role is used in wrong context.
#[tokio::test]
async fn invalid_context_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a thread protocol.
    // --------------------------------------------------
    let thread = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates 2 threads.
    // --------------------------------------------------
    let thread_1 = WriteBuilder::new()
        .data(Data::from(b"Thread 1".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread_1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let thread_2 = WriteBuilder::new()
        .data(Data::from(b"Thread 2".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread_2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a participant on the thread.
    // --------------------------------------------------
    let participant = WriteBuilder::new()
        .data(Data::from(b"Bob is a friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
            parent_context_id: thread_1.context_id.clone(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, participant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a chat message to thread 2.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Bob can read this because he is a participant".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/chat".to_string(),
            parent_context_id: thread_2.context_id.clone(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob uses his participant role to read the chat message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(chat.record_id))
        .protocol_role("thread/participant")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "unable to find record for role");
}

// Should disallow external party reads when grant has incorrect method scope.
#[tokio::test]
async fn invalid_grant_method() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"Bob can read this because I have granted him permission".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to write (not read) records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Write,
            protocol: "https://example.com/protocol/test".to_string(),
            limited_to: None,
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read his participant role record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "method is not within grant scope");
}

// Should allow reads of protocol records using grants with unrestricted scope.
#[tokio::test]
async fn unrestricted_grant() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: None,
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record without using the grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "no rule defined for action");

    // --------------------------------------------------
    // Bob reads the record using the grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should allow reads of protocol records with matching grant scope.
#[tokio::test]
async fn grant_protocol() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: Some(RecordsScope::ProtocolPath("foo".to_string())),
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record without using the grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "no rule defined for action");

    // --------------------------------------------------
    // Bob reads the record using the grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should not allow reads when grant scope does not match record protocol scope.
#[tokio::test]
async fn invalid_grant_protocol() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://a-different-protocol.com".to_string(),
            limited_to: None,
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record using the mismatching grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "scope protocol does not match write protocol");
}

// Should allow reading records within the context specified by the grant.
#[tokio::test]
async fn grant_context() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: Some(RecordsScope::ContextId(write.context_id.clone().unwrap())),
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads the record using the grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should not allow reading records within when grant context does not match.
#[tokio::test]
async fn invalid_grant_context() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: Some(RecordsScope::ContextId("somerandomgrant".to_string())),
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record using the mismatching grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "record not part of grant context");
}

// Should allow reading records in the grant protocol path.
#[tokio::test]
async fn grant_protocol_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: Some(RecordsScope::ProtocolPath("foo".to_string())),
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record using the mismatching grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should not allow reading records outside the grant protocol path.
#[tokio::test]
async fn invalid_grant_protocol_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a minimal protocol.
    // --------------------------------------------------
    let minimal = include_bytes!("../crates/dwn-test/protocols/minimal.json");
    let definition: Definition = serde_json::from_slice(minimal).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"minimal".to_vec()))
        .protocol(ProtocolSettings {
            protocol: "http://minimal.xyz".to_string(),
            protocol_path: "foo".to_string(),
            parent_context_id: None,
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice grants Bob permission to read records.
    // --------------------------------------------------
    let bob_grant = GrantBuilder::new()
        .granted_to(BOB_DID)
        .scope(Scope::Records {
            method: Method::Read,
            protocol: "http://minimal.xyz".to_string(),
            limited_to: Some(RecordsScope::ProtocolPath("different-protocol-path".to_string())),
        })
        .build(&alice_keyring)
        .await
        .expect("should create grant");
    let reply =
        endpoint::handle(ALICE_DID, bob_grant.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts to read the record using the mismatching grant.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(write.record_id))
        .permission_grant_id(bob_grant.record_id)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "grant and record protocol paths do not match");
}

// Should return a status of NotFound (404) when record does not exist.
#[tokio::test]
async fn record_not_found() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id("non-existent-record".to_string()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let Err(Error::NotFound(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be NotFound");
    };
    assert_eq!(e, "no matching record");
}

// Should return NotFound (404) when record has been deleted.
#[tokio::test]
async fn record_deleted() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes then  deletes a record.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let delete = DeleteBuilder::new()
        .record_id(&write.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");
    let reply = endpoint::handle(ALICE_DID, delete, &provider).await.expect("should read");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice attempts to read the deleted record.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    // TODO: convert to a NotFound error.
    // let Err(Error::NotFound(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
    //     panic!("should be NotFound");
    // };
    // assert_eq!(e, "no matching record");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::NOT_FOUND);
}

// Should return NotFound (404) when record data blocks have been deleted.
#[tokio::test]
async fn data_blocks_deleted() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record and then deletes its data from BlockStore.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);

    let write = WriteBuilder::new()
        .data(Data::from(data.to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // delete record's data
    BlockStore::delete(&provider, ALICE_DID, &write.descriptor.data_cid)
        .await
        .expect("should delete block");

    // --------------------------------------------------
    // Alice attempts to read the record with deleted data.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let Err(Error::NotFound(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be NotFound");
    };
    assert_eq!(e, "no data found");
}

// Should not get data from block store when record has `encoded_data`.
#[tokio::test]
async fn encoded_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record and then deletes data from BlockStore.
    // --------------------------------------------------
    let write = WriteBuilder::new()
        .data(Data::from(b"data small enough to be encoded".to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // deleting BlockStore data has no effect as the record uses encoded data
    BlockStore::delete(&provider, ALICE_DID, &write.descriptor.data_cid)
        .await
        .expect("should delete block");

    // --------------------------------------------------
    // Alice reads the record with encoded data.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply =
        endpoint::handle(ALICE_DID, read, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);
}

// Should get data from block store when record does not have `encoded_data`.
#[tokio::test]
async fn block_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record and then deletes its data from BlockStore.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice reads the record with block store data.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
    let Some(read_stream) = body.entry.data else {
        panic!("should have data");
    };
    assert_eq!(read_stream.compute_cid(), write_stream.compute_cid());
}

// Should decrypt flat-space schema-contained records using a derived key.
#[tokio::test]
async fn decrypt_schema() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let alice_kid = format!("{ALICE_DID}#z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX");

    // derive x25519 key from ed25519 key (Edwards -> Montgomery)
    let verifying_bytes: [u8; 32] =
        Base64UrlUnpadded::decode_vec(ALICE_VERIFYING_KEY).unwrap().try_into().unwrap();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&verifying_bytes).unwrap();
    let alice_public = x25519_dalek::PublicKey::from(verifying_key.to_montgomery().to_bytes());

    let schema = String::from("https://some-schema.com");
    let data_format = String::from("some/format");

    // --------------------------------------------------
    // Alice derives and issues participants' keys.
    // The keys are used for decrypting data for selected messages with each
    // key 'locked' to it's derivation scheme and path.
    //
    // N.B.
    // - the root private key is the owner's private key
    // - derived private keys are encrypted (using recipient's public key) and
    //   distributed to each recipient (out of band)
    // --------------------------------------------------
    // schema encryption key
    let schema_root = DerivedPrivateJwk {
        root_key_id: alice_kid.clone(),
        derivation_scheme: DerivationScheme::Schemas,
        derivation_path: None,
        derived_private_key: PrivateKeyJwk {
            public_key: PublicKeyJwk {
                kty: KeyType::Okp,
                crv: Curve::Ed25519,
                x: Base64UrlUnpadded::encode_string(alice_public.as_bytes()),
                ..PublicKeyJwk::default()
            },
            d: "8rmFFiUcTjjrL5mgBzWykaH39D64VD0mbDHwILvsu30".to_string(),
        },
    };

    let path = vec![DerivationScheme::Schemas.to_string(), schema.clone()];
    let schema_leaf = hd_key::derive_jwk(schema_root.clone(), &DerivationPath::Full(&path))
        .expect("should derive private key");
    let schema_public = schema_leaf.derived_private_key.public_key.clone();

    // data format encryption key
    let mut data_formats_root = schema_root.clone(); // same root as schema
    data_formats_root.derivation_scheme = DerivationScheme::DataFormats;
    let path = vec![DerivationScheme::DataFormats.to_string(), schema.clone(), data_format.clone()];
    let data_formats_leaf =
        hd_key::derive_jwk(data_formats_root.clone(), &DerivationPath::Full(&path))
            .expect("should derive private key");
    let data_formats_public = data_formats_leaf.derived_private_key.public_key.clone();

    // --------------------------------------------------
    // Alice writes a record with encrypted data.
    // --------------------------------------------------
    let options = EncryptOptions::new()
        .with_recipient(Recipient {
            key_id: alice_kid.clone(),
            public_key: schema_public,
            derivation_scheme: DerivationScheme::Schemas,
        })
        .with_recipient(Recipient {
            key_id: alice_kid.clone(),
            public_key: data_formats_public,
            derivation_scheme: DerivationScheme::DataFormats,
        });

    // generate data and encrypt
    let data = "hello world".as_bytes().to_vec();
    let (ciphertext, settings) = options.encrypt(&data).expect("should encrypt");

    // create Write record
    let write = WriteBuilder::new()
        .data(Data::from(ciphertext))
        .schema(schema)
        .data_format(&data_format)
        .encryption(settings)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice reads the record with encrypted data and decrypts it.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let write = body.entry.records_write.expect("should have write");

    let mut read_stream = body.entry.data.expect("should have data");
    let mut encrypted = Vec::new();
    read_stream.read_to_end(&mut encrypted).expect("should read data");

    // decrypt using schema descendant key
    let plaintext =
        decrypt(&encrypted, &write, &schema_leaf, &alice_keyring).await.expect("should decrypt");
    assert_eq!(plaintext, data);

    // decrypt using data format descendant key
    let plaintext = decrypt(&encrypted, &write, &data_formats_leaf, &alice_keyring)
        .await
        .expect("should decrypt");
    assert_eq!(plaintext, data);

    // decrypt using schema root key
    let plaintext =
        decrypt(&encrypted, &write, &schema_root, &alice_keyring).await.expect("should decrypt");
    assert_eq!(plaintext, data);

    // decrypt using data format root key
    let plaintext = decrypt(&encrypted, &write, &data_formats_root, &alice_keyring)
        .await
        .expect("should decrypt");
    assert_eq!(plaintext, data);

    // --------------------------------------------------
    // Check decryption fails using key derived from invalid path.
    // --------------------------------------------------
    let invalid_path = vec![DerivationScheme::DataFormats.to_string(), data_format];
    let invalid_key =
        hd_key::derive_jwk(data_formats_root.clone(), &DerivationPath::Full(&invalid_path))
            .expect("should derive private key");

    let Err(Error::BadRequest(_)) = decrypt(&encrypted, &write, &invalid_key, &alice_keyring).await
    else {
        panic!("should be BadRequest");
    };
}

// Should decrypt flat-space schemaless records using a derived key.
#[tokio::test]
async fn decrypt_schemaless() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice derives participants' keys.
    // --------------------------------------------------
    let alice_kid = format!("{ALICE_DID}#z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX");

    // derive x25519 key from ed25519 key (Edwards -> Montgomery)
    let verifying_bytes: [u8; 32] =
        Base64UrlUnpadded::decode_vec(ALICE_VERIFYING_KEY).unwrap().try_into().unwrap();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&verifying_bytes).unwrap();
    let alice_public = x25519_dalek::PublicKey::from(verifying_key.to_montgomery().to_bytes());

    let data_format = String::from("image/jpg");

    // encryption key
    let data_formats_root = DerivedPrivateJwk {
        root_key_id: alice_kid.clone(),
        derivation_scheme: DerivationScheme::DataFormats,
        derivation_path: None,
        derived_private_key: PrivateKeyJwk {
            public_key: PublicKeyJwk {
                kty: KeyType::Okp,
                crv: Curve::Ed25519,
                x: Base64UrlUnpadded::encode_string(alice_public.as_bytes()),
                ..PublicKeyJwk::default()
            },
            d: "8rmFFiUcTjjrL5mgBzWykaH39D64VD0mbDHwILvsu30".to_string(),
        },
    };

    let path = vec![DerivationScheme::DataFormats.to_string(), data_format.clone()];
    let data_formats_leaf =
        hd_key::derive_jwk(data_formats_root.clone(), &DerivationPath::Full(&path))
            .expect("should derive private key");
    let data_formats_public = data_formats_leaf.derived_private_key.public_key.clone();

    // --------------------------------------------------
    // Alice writes a record with encrypted data.
    // --------------------------------------------------
    // generate data and encrypt
    let data = "hello world".as_bytes().to_vec();

    let options = EncryptOptions::new().with_recipient(Recipient {
        key_id: alice_kid.clone(),
        public_key: data_formats_public,
        derivation_scheme: DerivationScheme::DataFormats,
    });

    let (ciphertext, settings) = options.encrypt(&data).expect("should encrypt");

    // create Write record
    let write = WriteBuilder::new()
        .data(Data::from(ciphertext))
        .data_format(&data_format)
        .encryption(settings)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice reads the record with encrypted data and decrypts it.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let write = body.entry.records_write.expect("should have write");

    let mut read_stream = body.entry.data.expect("should have data");
    let mut encrypted = Vec::new();
    read_stream.read_to_end(&mut encrypted).expect("should read data");

    // decrypt using schema descendant key
    let plaintext = decrypt(&encrypted, &write, &data_formats_root, &alice_keyring)
        .await
        .expect("should decrypt");
    assert_eq!(plaintext, data);
}

// Should only be able to decrypt records using the correct derived private key
// within a protocol-context derivation scheme.
#[tokio::test]
async fn decrypt_context() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice's keys.
    // --------------------------------------------------
    let alice_kid = format!("{ALICE_DID}#z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX");

    // derive x25519 key from ed25519 key (Edwards -> Montgomery)
    let verifying_bytes: [u8; 32] =
        Base64UrlUnpadded::decode_vec(ALICE_VERIFYING_KEY).unwrap().try_into().unwrap();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&verifying_bytes).unwrap();
    let alice_public = x25519_dalek::PublicKey::from(verifying_key.to_montgomery().to_bytes());

    let alice_private_jwk = PrivateKeyJwk {
        public_key: PublicKeyJwk {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: Base64UrlUnpadded::encode_string(alice_public.as_bytes()),
            ..PublicKeyJwk::default()
        },
        d: "8rmFFiUcTjjrL5mgBzWykaH39D64VD0mbDHwILvsu30".to_string(),
    };

    // --------------------------------------------------
    // Bob's keys.
    // --------------------------------------------------
    let bob_kid = format!("{BOB_DID}#z6MkqWGVUwMwt4ahxESTVg1gjvxZ4w4KkXomksSMdCB3eHeD");

    // derive x25519 key from ed25519 key (Edwards -> Montgomery)
    let verifying_bytes: [u8; 32] =
        Base64UrlUnpadded::decode_vec(BOB_VERIFYING_KEY).unwrap().try_into().unwrap();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&verifying_bytes).unwrap();
    let bob_public = x25519_dalek::PublicKey::from(verifying_key.to_montgomery().to_bytes());

    let bob_private_jwk = PrivateKeyJwk {
        public_key: PublicKeyJwk {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: Base64UrlUnpadded::encode_string(bob_public.as_bytes()),
            ..PublicKeyJwk::default()
        },
        d: "n8Rcm64tLob0nveDUuXzP-CnLmn3V11vRqk6E3FuKCo".to_string(),
    };

    // --------------------------------------------------
    // Alice configures the chat protocol with encryption.
    // --------------------------------------------------
    let chat = include_bytes!("../crates/dwn-test/protocols/chat.json");
    let definition: Definition = serde_json::from_slice(chat).expect("should deserialize");
    let definition = definition
        .add_encryption(&alice_kid, alice_private_jwk.clone())
        .expect("should add encryption");

    let configure_alice = ConfigureBuilder::new()
        .definition(definition)
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply = endpoint::handle(ALICE_DID, configure_alice, &provider)
        .await
        .expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob configures the chat protocol with encryption.
    // --------------------------------------------------
    let definition: Definition = serde_json::from_slice(chat).expect("should deserialize");
    let definition = definition
        .add_encryption(&bob_kid, bob_private_jwk.clone())
        .expect("should add encryption");

    let configure_bob = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&bob_keyring)
        .await
        .expect("should build");
    let reply = endpoint::handle(BOB_DID, configure_bob, &provider)
        .await
        .expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    //  Bob queries for Alice's chat protocol definition.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter("http://chat-protocol.xyz")
        .build(&bob_keyring)
        .await
        .expect("should build");

    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should match");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].authorization.author().unwrap(), ALICE_DID);

    // --------------------------------------------------
    //  Bob writes an initiating chat thread to ALice's web node.
    // --------------------------------------------------
    // generate data and encrypt
    let data = "Hello Alice".as_bytes().to_vec();
    let mut options = EncryptOptions::new().data(&data);
    let mut encrypted = options.encrypt2().expect("should encrypt");

    // create Write record
    let mut write = WriteBuilder::new()
        .data(Data::from(encrypted.ciphertext.clone()))
        .protocol(ProtocolSettings {
            protocol: "http://chat-protocol.xyz".to_string(),
            protocol_path: "thread".to_string(),
            parent_context_id: None,
        })
        .schema("thread")
        .data_format("application/json")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");

    // get the rule set for the protocol path
    let rule_set = definition.structure.get("thread").unwrap();
    let encryption = rule_set.encryption.as_ref().unwrap();

    // protocol path derived public key
    encrypted = encrypted.add_recipient(Recipient {
        key_id: encryption.root_key_id.clone(),
        public_key: encryption.public_key_jwk.clone(),
        derivation_scheme: DerivationScheme::ProtocolPath,
    });

    // protocol context derived public key
    let bob_root = DerivedPrivateJwk {
        root_key_id: bob_kid.clone(),
        derivation_scheme: DerivationScheme::ProtocolContext,
        derivation_path: None,
        derived_private_key: bob_private_jwk.clone(),
    };

    let context_id = write.context_id.clone().unwrap();
    let context_path = [DerivationScheme::ProtocolContext.to_string(), context_id.clone()];
    let context_jwk = hd_key::derive_jwk(bob_root.clone(), &DerivationPath::Full(&context_path))
        .expect("should derive key");

    encrypted = encrypted.add_recipient(Recipient {
        key_id: bob_kid.clone(),
        public_key: context_jwk.derived_private_key.public_key.clone(),
        derivation_scheme: DerivationScheme::ProtocolContext,
    });

    // generate data and encrypt
    let encryption = encrypted.finalize().expect("should encrypt");

    // finalize Write record
    write.encryption = Some(encryption);
    write.sign_as_author(None, None, &bob_keyring).await.expect("should sign");

    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    //  Bob also writes the message to his web node.
    // --------------------------------------------------
    let reply = endpoint::handle(BOB_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Anyone with the protocol context derived private key should be able to
    // decrypt the message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let write = body.entry.records_write.expect("should have write");

    let mut read_stream = body.entry.data.expect("should have data");
    let mut encrypted = Vec::new();
    read_stream.read_to_end(&mut encrypted).expect("should read data");

    // decrypt using context-derived descendant key
    let plaintext =
        decrypt(&encrypted, &write, &context_jwk, &alice_keyring).await.expect("should decrypt");
    assert_eq!(plaintext, data);

    // --------------------------------------------------
    // Alice sends Bob an encrypted message using the protocol
    // context public key derived above.
    // --------------------------------------------------
    // generate data and encrypt
    let data = "Hello Bob".as_bytes().to_vec();
    let mut options = EncryptOptions::new().data(&data);
    let mut encrypted = options.encrypt2().expect("should encrypt");

    // create Write record
    let mut write = WriteBuilder::new()
        .data(Data::from(encrypted.ciphertext.clone()))
        .protocol(ProtocolSettings {
            protocol: "http://chat-protocol.xyz".to_string(),
            protocol_path: "thread/message".to_string(),
            parent_context_id: Some(context_id.clone()),
        })
        .schema("message")
        .data_format("application/json")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");

    // get the rule set for the protocol path
    let rule_set = definition.structure.get("thread").unwrap();
    let _encryption = rule_set.encryption.as_ref().unwrap();

    let context_id = write.context_id.clone().unwrap();
    let segment_1 = context_id.split("/").collect::<Vec<&str>>()[0];
    let context_path = [DerivationScheme::ProtocolContext.to_string(), segment_1.to_string()];
    let context_jwk = hd_key::derive_jwk(bob_root.clone(), &DerivationPath::Full(&context_path))
        .expect("should derive key");

    encrypted = encrypted.add_recipient(Recipient {
        key_id: bob_kid.clone(),
        public_key: context_jwk.derived_private_key.public_key.clone(),
        derivation_scheme: DerivationScheme::ProtocolContext,
    });

    // generate data and encrypt
    let encryption = encrypted.finalize().expect("should encrypt");

    // finalize Write record
    write.encryption = Some(encryption);
    write.sign_as_author(None, None, &bob_keyring).await.expect("should sign");

    let reply = endpoint::handle(BOB_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob reads Alice's message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(BOB_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let write = body.entry.records_write.expect("should have write");

    let mut read_stream = body.entry.data.expect("should have data");
    let mut encrypted = Vec::new();
    read_stream.read_to_end(&mut encrypted).expect("should read data");

    // decrypt using context-derived descendant key
    let plaintext =
        decrypt(&encrypted, &write, &context_jwk, &bob_keyring).await.expect("should decrypt");
    assert_eq!(plaintext, data);
}

// Should only be able to decrypt records using the correct derived private key
// within a protocol derivation scheme.
#[tokio::test]
async fn decrypt_protocol() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice's keys.
    // --------------------------------------------------
    let alice_kid = format!("{ALICE_DID}#z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX");

    // derive x25519 key from ed25519 key (Edwards -> Montgomery)
    let verifying_bytes: [u8; 32] =
        Base64UrlUnpadded::decode_vec(ALICE_VERIFYING_KEY).unwrap().try_into().unwrap();
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&verifying_bytes).unwrap();
    let alice_public = x25519_dalek::PublicKey::from(verifying_key.to_montgomery().to_bytes());

    let alice_private_jwk = PrivateKeyJwk {
        public_key: PublicKeyJwk {
            kty: KeyType::Okp,
            crv: Curve::Ed25519,
            x: Base64UrlUnpadded::encode_string(alice_public.as_bytes()),
            ..PublicKeyJwk::default()
        },
        d: "8rmFFiUcTjjrL5mgBzWykaH39D64VD0mbDHwILvsu30".to_string(),
    };

    // --------------------------------------------------
    // Alice configures the email protocol with encryption.
    // --------------------------------------------------
    let email = include_bytes!("../crates/dwn-test/protocols/email.json");
    let definition: Definition = serde_json::from_slice(email).expect("should deserialize");
    let definition = definition
        .add_encryption(&alice_kid, alice_private_jwk.clone())
        .expect("should add encryption");

    let email = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, email, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    //  Bob queries for Alice's email protocol definition.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter("http://email-protocol.xyz")
        .build(&bob_keyring)
        .await
        .expect("should build");

    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should match");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].authorization.author().unwrap(), ALICE_DID);

    // --------------------------------------------------
    //  Bob writes an encrypted email to Alice.
    // --------------------------------------------------
    // generate data and encrypt
    let data = "Hello Alice".as_bytes().to_vec();
    let mut options = EncryptOptions::new().data(&data);
    let mut encrypted = options.encrypt2().expect("should encrypt");
    let ciphertext = encrypted.ciphertext.clone();

    // get the rule set for the protocol path
    let rule_set = definition.structure.get("email").unwrap();
    let encryption = rule_set.encryption.as_ref().unwrap();

    // protocol path derived public key
    encrypted = encrypted.add_recipient(Recipient {
        key_id: alice_kid.clone(),
        public_key: encryption.public_key_jwk.clone(),
        derivation_scheme: DerivationScheme::ProtocolPath,
    });

    // generate data and encrypt
    let encryption = encrypted.finalize().expect("should encrypt");

    // create Write record
    let write = WriteBuilder::new()
        .data(Data::from(ciphertext))
        .protocol(ProtocolSettings {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
            parent_context_id: None,
        })
        .schema("email")
        .data_format("text/plain")
        .encryption(encryption)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");

    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    //  Alice read Bob's message.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let write = body.entry.records_write.expect("should have write");

    let mut read_stream = body.entry.data.expect("should have data");
    let mut encrypted = Vec::new();
    read_stream.read_to_end(&mut encrypted).expect("should read data");

    // decrypt using her private key
    let alice_jwk = DerivedPrivateJwk {
        root_key_id: alice_kid.clone(),
        derivation_scheme: DerivationScheme::ProtocolPath,
        derivation_path: None,
        derived_private_key: alice_private_jwk.clone(),
    };

    let plaintext =
        decrypt(&encrypted, &write, &alice_jwk, &bob_keyring).await.expect("should decrypt");
    assert_eq!(plaintext, data);
}

// Should return Unauthorized (401) for invalid signatures.
#[tokio::test]
async fn invalid_signature() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let mut read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id("somerecordid".to_string()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");

    read.authorization.as_mut().unwrap().signature.signatures[0].signature =
        "badsignature".to_string();

    let Err(Error::Unauthorized(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be Unauthorized");
    };
    assert!(e.starts_with("failed to authenticate: "));
}

// Should return BadRequest (400) for unparsable messages.
#[tokio::test]
async fn invalid_message() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let mut read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id("somerecordid".to_string()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");

    read.descriptor.filter = RecordsFilter::default();

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, read, &provider).await else {
        panic!("should be BadRequest");
    };
    assert!(e.starts_with("validation failed for "));
}
