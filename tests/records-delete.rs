//! Records Delete

use http::StatusCode;
use serde_json::json;
use test_utils::store::ProviderImpl;
use vercre_dwn::data::DataStream;
use vercre_dwn::endpoint;
use vercre_dwn::provider::KeyStore;
use vercre_dwn::records::{DeleteBuilder, QueryBuilder, RecordsFilter, WriteBuilder, WriteData};

const ALICE_DID: &str = "did:key:z6Mkj8Jr1rg3YjVWWhg7ahEYJibqhjBgZt1pDCbT4Lv7D4HX";

// Successfully delete a record and then fail when attempting to delete it again.
#[tokio::test]
async fn delete_record() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a message to her web node
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
    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Ensure the record was written.
    // --------------------------------------------------
    let filter = RecordsFilter {
        record_id: Some(write.record_id.clone()),
        ..RecordsFilter::default()
    };
    let query = QueryBuilder::new()
        .filter(filter)
        .build(&alice_keyring)
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, query.clone(), &provider).await.expect("should read");
    assert_eq!(reply.status.code, StatusCode::OK);

    // --------------------------------------------------
    // Delete the record.
    // --------------------------------------------------
    let delete = DeleteBuilder::new()
        .record_id(&write.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");

    let reply = endpoint::handle(ALICE_DID, delete, &provider).await.expect("should read");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Ensure record doesn't appear in query results.
    // --------------------------------------------------
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should read");
    assert_eq!(reply.status.code, StatusCode::OK);
    assert!(reply.body.unwrap().entries.is_none());

    // --------------------------------------------------
    // Deleting the same record should fail.
    // --------------------------------------------------
    let delete = DeleteBuilder::new()
        .record_id(&write.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");
    let err = endpoint::handle(ALICE_DID, delete, &provider).await.expect_err("should be 404");
    assert_eq!(
        err.to_json(),
        json!({"code": 404, "detail": "cannot delete a `RecordsDelete` record"})
    );
}
