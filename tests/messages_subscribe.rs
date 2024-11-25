//! Messages Subscribe

use futures::StreamExt;
use http::StatusCode;
use serde_json::json;
use test_utils::store::ProviderImpl;
use vercre_dwn::data::DataStream;
use vercre_dwn::messages::{MessagesFilter, QueryBuilder, SubscribeBuilder};
use vercre_dwn::provider::KeyStore;
use vercre_dwn::records::{WriteBuilder, WriteData};
use vercre_dwn::{endpoint, Interface, Message};

const ALICE_DID: &str = "did:key:z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX";

// The owner should be able to to subscribe their own event stream
#[tokio::test]
async fn owner_events() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice subscribes to own event stream.
    // --------------------------------------------------
    let filter = MessagesFilter::new().interface(Interface::Records);
    let subscribe = SubscribeBuilder::new()
        .add_filter(filter)
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, subscribe, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::OK);

    let Some(mut subscribe_reply) = reply.body else {
        panic!("unexpected reply: {:?}", reply);
    };

    // --------------------------------------------------
    // Alice writes a record.
    // --------------------------------------------------
    let data = serde_json::to_vec(&json!({
        "message": "test record write",
    }))
    .expect("should serialize");

    let write = WriteBuilder::new()
        .data(WriteData::Reader {
            reader: DataStream::from(data),
        })
        .build(&alice_keyring)
        .await
        .expect("should create write");

    let message_cid = write.cid().expect("should have cid");

    let reply = endpoint::handle(ALICE_DID, write, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Ensure the RecordsWrite event exists.
    // --------------------------------------------------
    let query = QueryBuilder::new().build(&alice_keyring).await.expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should query");
    assert_eq!(reply.status.code, StatusCode::OK);

    let query_reply = reply.body.expect("should have reply");
    let entries = query_reply.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], message_cid);

    // --------------------------------------------------
    // The subscriber should have a matching write event.
    // --------------------------------------------------
    // if let Some(event) = subscribe_reply.subscription.next().await {
    //     assert_eq!(message_cid, event.cid().unwrap());
    // }

    let mut found = false;
    while let Some(event) = subscribe_reply.subscription.next().await {
        println!("{}", event.cid().unwrap());
        if message_cid == event.cid().unwrap() {
            found = true;
            break;
        }
    }
    assert!(found);
}
