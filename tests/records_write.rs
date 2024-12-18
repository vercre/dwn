//! Records Write

use base64ct::{Base64UrlUnpadded, Encoding};
use dwn_test::key_store::ALICE_DID;
use dwn_test::provider::ProviderImpl;
use http::StatusCode;
use serde_json::json;
use vercre_dwn::data::DataStream;
use vercre_dwn::endpoint;
use vercre_dwn::provider::KeyStore;
use vercre_dwn::records::{QueryBuilder, RecordsFilter, WriteBuilder, WriteData};

// The owner should be able to to subscribe their own event stream
#[tokio::test]
async fn overwrite_older() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let data = serde_json::to_vec(&json!({
        "message": "a new write record",
    }))
    .expect("should serialize");
    let encoded_data = Base64UrlUnpadded::encode_string(&data);

    let initial_write = WriteBuilder::new()
        .data(WriteData::Reader(DataStream::from(data.clone())))
        .build(&alice_keyring)
        .await
        .expect("should create write");
    let record_id = initial_write.record_id.clone();

    let reply = endpoint::handle(ALICE_DID, initial_write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record was created.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(record_id))
        .build(&alice_keyring)
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(encoded_data));

    // --------------------------------------------------
    // Update the existing record.
    // --------------------------------------------------
    let data = serde_json::to_vec(&json!({
        "message": "updated write record",
    }))
    .expect("should serialize");
    let encoded_data = Base64UrlUnpadded::encode_string(&data);

    let write = WriteBuilder::new()
        .data(WriteData::Reader(DataStream::from(data)))
        .existing_write(initial_write)
        .build(&alice_keyring)
        .await
        .expect("should create write");
    let record_id = write.record_id.clone();

    let reply = endpoint::handle(ALICE_DID, write.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the updated record overwrote the original.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(record_id))
        .build(&alice_keyring)
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(encoded_data));

    // --------------------------------------------------
    // Attempt to overwrite the latest record with an older version.
    // --------------------------------------------------
    // // try to write the older message to store again and verify that it is not accepted
    // const thirdRecordsWriteReply =
    //   await dwn.processMessage(tenant, recordsWriteMessageData.message, { dataStream: recordsWriteMessageData.dataStream });
    // expect(thirdRecordsWriteReply.status.code).to.equal(409); // expecting to fail

    // // expecting unchanged
    // const thirdRecordsQueryReply = await dwn.processMessage(tenant, recordsQueryMessageData.message);
    // expect(thirdRecordsQueryReply.status.code).to.equal(200);
    // expect(thirdRecordsQueryReply.entries?.length).to.equal(1);
    // expect(thirdRecordsQueryReply.entries![0].encodedData).to.equal(newDataEncoded);
}
