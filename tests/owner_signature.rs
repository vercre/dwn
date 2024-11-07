//! Author Delegated Grant
//!
//! This test demonstrates how a web node owner can delegate permission to
//! another entity to perform an action on their behalf. In this case, Alice
//! grants Bob the ability to configure a protocol on her behalf.

use insta::assert_yaml_snapshot as assert_snapshot;
use serde_json::json;
use test_utils::store::ProviderImpl;
use vercre_dwn::provider::KeyStore;
use vercre_dwn::records::{ReadBuilder, RecordsFilter, WriteBuilder, WriteData};
use vercre_dwn::service::Reply;

const ALICE_DID: &str = "did:key:z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX";
const BOB_DID: &str = "did:key:z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX";

// Use owner signature for authorization when it is provided.
#[tokio::test]
async fn flat_space() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Bob writes a message to his web node
    // --------------------------------------------------
    let data = serde_json::to_vec(&json!({
        "message": "test record write",
    }))
    .expect("should serialize");

    let write = WriteBuilder::new()
        .data(WriteData::Bytes { data })
        .published(true)
        .build(&bob_keyring)
        .await
        .expect("should create write");

    let reply = vercre_dwn::handle_message(BOB_DID, write.clone(), provider.clone())
        .await
        .expect("should write");
    assert_eq!(reply.status().code, 204);

    // --------------------------------------------------
    // Alice fetches the message from Bob's web node
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter {
            record_id: Some(write.record_id),
            ..RecordsFilter::default()
        })
        .build(&alice_keyring)
        .await
        .expect("should create write");

    let reply =
        vercre_dwn::handle_message(BOB_DID, read, provider.clone()).await.expect("should write");
    assert_eq!(reply.status().code, 200);

    assert_snapshot!("read", reply, {
        ".recordsWrite.recordId" => "[recordId]",
        ".recordsWrite.descriptor.messageTimestamp" => "[messageTimestamp]",
        ".recordsWrite.descriptor.dateCreated" => "[dateCreated]",
        ".recordsWrite.descriptor.datePublished" => "[datePublished]",
        ".recordsWrite.authorization.signature.payload" => "[payload]",
        ".recordsWrite.authorization.signature.signatures[0].signature" => "[signature]",
        ".recordsWrite.attestation.payload" => "[payload]",
        ".recordsWrite.attestation.signatures[0].signature" => "[signature]",
    });

    // --------------------------------------------------
    // Bob configures the email protocol on Alice's behalf
    // --------------------------------------------------
    // let builder = GrantBuilder::new()
    //     .granted_to(BOB_DID)
    //     .request_id("grant_id_1")
    //     .description("Allow Bob to configure any protocol")
    //     .delegated(true)
    //     .scope(Interface::Protocols, Method::Configure, None);

    // let grant_to_bob = builder.build(&alice_keyring).await.expect("should create grant");

    // let email_json = include_bytes!("protocols/email.json");
    // let email_proto: Definition =
    //     serde_json::from_slice(email_json).expect("should deserialize");

    // let configure = ConfigureBuilder::new()
    //     .definition(email_proto.clone())
    //     .delegated_grant(grant_to_bob)
    //     .build(&bob_keyring)
    //     .await
    //     .expect("should build");

    // let message = Message::ProtocolsConfigure(configure);
    // let reply = vercre_dwn::handle_message(ALICE_DID, message, provider.clone())
    //     .await
    //     .expect("should configure protocol");

    // let Reply::ProtocolsConfigure(reply) = reply else {
    //     panic!("unexpected reply: {:?}", reply);
    // };

    // assert_eq!(reply.status.code, 202);

    // --------------------------------------------------
    // Alice fetches the email protocol configured by Bob
    // --------------------------------------------------
    // let query = QueryBuilder::new()
    //     .filter(email_proto.protocol)
    //     .build(&alice_keyring)
    //     .await
    //     .expect("should build");

    // let message = Message::ProtocolsQuery(query);
    // let reply = vercre_dwn::handle_message(ALICE_DID, message, provider.clone())
    //     .await
    //     .expect("should find protocol");

    // let Reply::ProtocolsQuery(reply) = reply else {
    //     panic!("unexpected reply: {:?}", reply);
    // };

    // assert_eq!(reply.status.code, 200);
}
