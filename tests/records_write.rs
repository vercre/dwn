//! Records Write

use base64ct::{Base64UrlUnpadded, Encoding};
use chrono::{DateTime, Duration, Utc};
use dwn_test::key_store::{
    ALICE_DID, APP_DID as ISSUER_DID, BOB_DID, CAROL_DID, CAROL_DID as FAKE_DID,
};
use dwn_test::provider::ProviderImpl;
use http::StatusCode;
use rand::RngCore;
use vercre_dwn::data::{DataStream, MAX_ENCODED_SIZE};
use vercre_dwn::messages::{self, MessagesFilter};
use vercre_dwn::protocols::{ConfigureBuilder, Definition};
use vercre_dwn::provider::{EventLog, KeyStore};
use vercre_dwn::records::{
    Data, DeleteBuilder, QueryBuilder, ReadBuilder, RecordsFilter, WriteBuilder, WriteProtocol,
    entry_id,
};
use vercre_dwn::store::MessagesQuery;
use vercre_dwn::{Error, Interface, Message, endpoint};

// // Should handle pre-processing errors
// #[tokio::test]
// async fn pre_process() {}

// Should be able to update existing record when update has a later `message_timestamp`.
#[tokio::test]
async fn update_older() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let data = b"a new write record";

    let initial = WriteBuilder::new()
        .data(Data::from(data.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record was created.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(data)));

    // --------------------------------------------------
    // Update the existing record.
    // --------------------------------------------------
    let data = b"updated write record";

    let update = WriteBuilder::from(initial.clone())
        .data(Data::from(data.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the updated record overwrote the original.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&update.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(data)));

    // --------------------------------------------------
    // Attempt to overwrite the latest record with an older version.
    // --------------------------------------------------
    let Err(Error::Conflict(e)) = endpoint::handle(ALICE_DID, initial, &provider).await else {
        panic!("should be Conflict");
    };
    assert_eq!(e, "a more recent update exists");

    // --------------------------------------------------
    // Verify the latest update remains unchanged.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(update.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(data)));
}

// Should be able to update existing record with identical message_timestamp
// only when message CID is larger than the existing one.
#[tokio::test]
async fn update_smaller_cid() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"a new write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Create 2 records with the same `message_timestamp`.
    // --------------------------------------------------
    // let message_timestamp = DateTime::parse_from_rfc3339("2024-12-31T00:00:00-00:00").unwrap();
    let message_timestamp = initial.descriptor.base.message_timestamp + Duration::seconds(1);

    let write_1 = WriteBuilder::from(initial.clone())
        .data(Data::from(b"message 1".to_vec()))
        .message_timestamp(message_timestamp.into())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    let write_2 = WriteBuilder::from(initial.clone())
        .data(Data::from(b"message 2".to_vec()))
        .message_timestamp(message_timestamp.into())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // determine the order of the writes by CID size
    let mut sorted = vec![write_1.clone(), write_2.clone()];
    sorted.sort_by(|a, b| a.cid().unwrap().cmp(&b.cid().unwrap()));

    // --------------------------------------------------
    // Update the initial record with the first update (ordered by CID size).
    // --------------------------------------------------
    let reply =
        endpoint::handle(ALICE_DID, sorted[0].clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // verify update
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.descriptor.data_cid, sorted[0].descriptor.data_cid);

    // --------------------------------------------------
    // Apply the second update (ordered by CID size).
    // --------------------------------------------------
    let reply =
        endpoint::handle(ALICE_DID, sorted[1].clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // verify update
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.descriptor.data_cid, sorted[1].descriptor.data_cid);

    // --------------------------------------------------
    // Attempt to update using the first update (smaller CID) update and fail.
    // --------------------------------------------------
    let Err(Error::Conflict(e)) = endpoint::handle(ALICE_DID, sorted[0].clone(), &provider).await
    else {
        panic!("should be Conflict");
    };
    assert_eq!(e, "an update with a larger CID already exists");
}

// Should allow data format of a flat-space record to be updated to any value.
#[tokio::test]
async fn update_flat_space() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"a new write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Update the record with a new data format.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .data(Data::from(b"update write record".to_vec()))
        .data_format("a-new-data-format")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the data format has been updated.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.descriptor.data_format, update.descriptor.data_format);
}

// Should not allow immutable properties to be updated.
#[tokio::test]
async fn immutable_unchanged() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify `date_created` cannot be updated.
    // --------------------------------------------------
    let date_created = Utc::now();

    let update = WriteBuilder::new()
        .record_id(initial.record_id.clone())
        .date_created(date_created)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update.clone(), &provider).await
    else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "immutable properties do not match");

    // --------------------------------------------------
    // Verify `schema` cannot be updated.
    // --------------------------------------------------
    let update = WriteBuilder::new()
        .record_id(initial.record_id.clone())
        .schema("new-schema")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update.clone(), &provider).await
    else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "immutable properties do not match");
}

// Should inherit data from previous write when `data_cid` and `data_size`
// match and no data stream is provided.
#[tokio::test]
async fn inherit_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Update the record, providing data to calculate CID and size, but without
    // adding to block store.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the initial write and it's data are still available.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&update.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(b"some data")));
}

// ln 367: Should allow an initial write without data.
#[tokio::test]
async fn initial_no_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record with no data.
    // --------------------------------------------------
    let initial =
        WriteBuilder::new().sign(&alice_keyring).build().await.expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::NO_CONTENT);

    // --------------------------------------------------
    // Verify the record cannot be queried for.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);
    assert!(reply.body.is_none());

    // --------------------------------------------------
    // Update the record, adding data.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .data(Data::from(b"update write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the data format has been updated.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"update write record"))
    );
}

// ln 409: Should not allow a record to be updated without data.
#[tokio::test]
async fn update_no_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"some data".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Update the record, providing data to calculate CID and size, but without
    // setting `data_stream`.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .data(Data::Bytes(b"update write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update.clone(), &provider).await
    else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "data CID does not match descriptor `data_cid`");

    // --------------------------------------------------
    // Verify the initial write and it's data are still available.
    // --------------------------------------------------
    let read = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(b"some data")));
}

// Should inherit data from previous writes when data size greater than
// `encoded_data` threshold.
#[tokio::test]
async fn retain_large_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record with a lot of data.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let initial = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Update the record but not data.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the initial write's data is still available.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
    let read_stream = body.entry.data.expect("should have data");
    assert_eq!(read_stream.buffer, data.to_vec());
}

// Should inherit data from previous writes when data size less than
// `encoded_data` threshold.
#[tokio::test]
async fn retain_small_data() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a record with a lot of data.
    // --------------------------------------------------
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let initial = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Update the record but not data.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the initial write's data is still available.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");

    let reply = endpoint::handle(ALICE_DID, read.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    assert!(body.entry.records_write.is_some());
    let read_stream = body.entry.data.expect("should have data");
    assert_eq!(read_stream.buffer, data.to_vec());
}

// Should fail when data size greater than `encoded_data` threshold and
// descriptor `data_size` is larger than data size.
#[tokio::test]
async fn large_data_size_larger() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a lot of data and then change the `data_size`.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data size
    write.descriptor.data_size = MAX_ENCODED_SIZE + 100;
    write.record_id = entry_id(&write.descriptor, ALICE_DID).expect("should create record ID");

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data size does not match message `data_size`");
}

// Should fail when data size less than `encoded_data` threshold and descriptor
// `data_size` is larger than `encoded_data` threshold.
#[tokio::test]
async fn small_data_size_larger() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a small amount of data and then change the `data_size`.
    // --------------------------------------------------
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data size
    write.descriptor.data_size = MAX_ENCODED_SIZE + 100;
    write.record_id = entry_id(&write.descriptor, ALICE_DID).expect("should create record ID");

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data size does not match message `data_size`");
}

// Should fail when data size greater than `encoded_data` threshold and
// descriptor `data_size` is smaller than threshold.
#[tokio::test]
async fn large_data_size_smaller() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a lot of data and then change the `data_size`.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data size
    write.descriptor.data_size = 1;
    write.record_id = entry_id(&write.descriptor, ALICE_DID).expect("should create record ID");

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data size does not match message `data_size`");
}

// Should fail when data size less than `encoded_data` threshold and descriptor
// `data_size` is smaller than actual data size.
#[tokio::test]
async fn small_data_size_smaller() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a small amount of data and then change the `data_size`.
    // --------------------------------------------------
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data size and recalculate the `record_id`
    write.descriptor.data_size = 1;
    write.record_id = entry_id(&write.descriptor, ALICE_DID).expect("should create record ID");

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data size does not match message `data_size`");
}

// Should fail when data size greater than `encoded_data` threshold and
// descriptor `data_cid` is incorrect.
#[tokio::test]
async fn large_data_cid_larger() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a lot of data and then change the `data_cid`.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());
    write.data_stream = Some(write_stream);

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data CID does not match message `data_cid`");
}

// Should fail when data size less than `encoded_data` threshold and descriptor
// `data_cid` is incorrect.
#[tokio::test]
async fn small_data_cid_larger() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a small amount of data and then change the `data_cid`.
    // --------------------------------------------------
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());
    write.data_stream = Some(write_stream);

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data CID does not match message `data_cid`");
}

// Should fail when data size greater than `encoded_data` threshold and
// descriptor `data_cid` is incorrect.
#[tokio::test]
async fn large_data_cid_smaller() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a lot of data and then change the `data_cid`.
    // --------------------------------------------------
    let mut data = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());
    write.data_stream = Some(write_stream);

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data CID does not match message `data_cid`");
}

// Should fail when data size less than `encoded_data` threshold and descriptor
// `data_cid` is incorrect.
#[tokio::test]
async fn small_data_cid_smaller() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Writes a record with a small amount of data and then change the `data_cid`.
    // --------------------------------------------------
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());

    let mut write = WriteBuilder::new()
        .data(Data::Stream(write_stream.clone()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    let mut data = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data);
    let write_stream = DataStream::from(data.to_vec());
    write.data_stream = Some(write_stream);

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, write, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "actual data CID does not match message `data_cid`");
}

// Should prevent accessing data by referencing a different`data_cid` in an update.
#[tokio::test]
async fn alter_data_cid_larger() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write 2 records.
    // --------------------------------------------------
    // record 1
    let mut data_1 = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data_1);

    let write_1 = WriteBuilder::new()
        .data(Data::from(data_1.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // record 2
    let mut data_2 = [0u8; MAX_ENCODED_SIZE + 10];
    rand::thread_rng().fill_bytes(&mut data_2);

    let write_2 = WriteBuilder::new()
        .data(Data::from(data_2.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Attempt to update record 2 to reference record 1's data.
    // --------------------------------------------------
    let mut update = WriteBuilder::from(write_2.clone())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    update.descriptor.data_cid = write_1.descriptor.data_cid;
    update.descriptor.data_size = write_1.descriptor.data_size;

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "data CID does not match descriptor `data_cid`");

    // --------------------------------------------------
    // Verify record still has original data.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write_2.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let data = body.entry.data.expect("should have data");
    assert_eq!(data.buffer, data_2.to_vec());
}

// Should prevent accessing data by referencing a different`data_cid` in an update.
#[tokio::test]
async fn alter_data_cid_smaller() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write 2 records.
    // --------------------------------------------------
    // record 1
    let mut data_1 = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data_1);

    let write_1 = WriteBuilder::new()
        .data(Data::from(data_1.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // record 2
    let mut data_2 = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut data_2);

    let write_2 = WriteBuilder::new()
        .data(Data::from(data_2.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, write_2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Attempt to update record 2 to reference record 1's data.
    // --------------------------------------------------
    let mut update = WriteBuilder::from(write_2.clone())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    // alter the data CID
    update.descriptor.data_cid = write_1.descriptor.data_cid;
    update.descriptor.data_size = write_1.descriptor.data_size;

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "data CID does not match descriptor `data_cid`");

    // --------------------------------------------------
    // Verify record still has original data.
    // --------------------------------------------------
    let read = ReadBuilder::new()
        .filter(RecordsFilter::new().record_id(&write_2.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, read, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let data = body.entry.data.expect("should have data");
    assert_eq!(data.buffer, data_2.to_vec());
}

// Should allow updates without specifying `data` or `date_published`.
#[tokio::test]
async fn update_published_no_date() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify `date_created` cannot be updated.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record's `published` state has been updated.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .build()
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"new write record"))
    );
}

// Should conserve `published` state when updating using an existing Write record.
#[tokio::test]
async fn update_published() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify `date_created` cannot be updated.
    // --------------------------------------------------
    let update = WriteBuilder::from(initial.clone())
        .data(Data::from(b"update write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record's `published` state has been updated.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&initial.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.descriptor.published, Some(true));
    assert_eq!(
        entries[0].write.descriptor.date_published.unwrap().timestamp_micros(),
        initial.descriptor.date_published.unwrap().timestamp_micros()
    );
}

// Should fail when updating a record but its initial write cannot be found.
#[tokio::test]
async fn no_initial_write() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .record_id("bafkreihs5gnovjoqueffglvevvohpgts3aj5ykgmlqm7quuotujxtxtp7f")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, initial, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "initial write not found");
}

// Should fail when creating a record if `date_created` and `message_timestamp`
// do not match.
#[tokio::test]
async fn create_date_mismatch() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let created = DateTime::parse_from_rfc3339("2025-01-01T00:00:00-00:00").unwrap();

    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .date_created(created.into())
        .message_timestamp(Utc::now())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, initial, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "`message_timestamp` and `date_created` do not match");
}

// Should fail when creating a record with an invalid `context_id`.
#[tokio::test]
async fn invalid_context_id() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    let mut initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");

    initial.context_id =
        Some("bafkreihs5gnovjoqueffglvevvohpgts3aj5ykgmlqm7quuotujxtxtp7f".to_string());

    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, initial, &provider).await else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "invalid `context_id`");
}

// Should log an event on initial write.
#[tokio::test]
async fn log_initial_write() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record.
    // --------------------------------------------------
    let initial = WriteBuilder::new()
        .data(Data::from(b"new write record".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify an event was logged.
    // --------------------------------------------------
    let query = messages::QueryBuilder::new()
        .add_filter(MessagesFilter::new().interface(Interface::Records))
        .build(&alice_keyring)
        .await
        .expect("should create query");

    let query = MessagesQuery::from(query);
    let (events, _) =
        EventLog::query(&provider, ALICE_DID, &query.into()).await.expect("should fetch");
    assert_eq!(events.len(), 1);
}

// Should only ever retain (at most) the initial and most recent writes.
#[tokio::test]
async fn retain_two_writes() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Write a record and 2 updates.
    // --------------------------------------------------
    let data = b"a new write record";
    let initial = WriteBuilder::new()
        .data(Data::from(data.to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, initial.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let update1 = WriteBuilder::from(initial.clone())
        .published(true)
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, update1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    let update2 = WriteBuilder::from(initial.clone())
        .date_published(Utc::now())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, update2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify only the initial write and latest update remain.
    // --------------------------------------------------
    let query = messages::QueryBuilder::new()
        .add_filter(MessagesFilter::new().interface(Interface::Records))
        .build(&alice_keyring)
        .await
        .expect("should create query");

    let query = MessagesQuery::from(query);
    let (events, _) =
        EventLog::query(&provider, ALICE_DID, &query.into()).await.expect("should fetch");
    assert_eq!(events.len(), 2);

    assert_eq!(events[0].cid(), initial.cid());
    assert_eq!(events[1].cid(), update2.cid());
}

// Should allow anyone to create a record using the "anyone create" rule.
#[tokio::test]
async fn anyone_create() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

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
    // Bob writes an email.
    // --------------------------------------------------
    let email_data = b"Hello Alice";
    let email = WriteBuilder::new()
        .data(Data::from(email_data.to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://email-protocol.xyz".to_string(),
            protocol_path: "email".to_string(),
        })
        .schema("email")
        .data_format("text/plain")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, email.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice queries for the email from Bob.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&email.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].write.encoded_data, Some(Base64UrlUnpadded::encode_string(email_data)));
}

// Should allow anyone to create a record using the "anyone co-update" rule.
#[tokio::test]
async fn anyone_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a collaboration protocol.
    // --------------------------------------------------
    let collab = include_bytes!("../crates/dwn-test/protocols/anyone-collaborate.json");
    let definition: Definition = serde_json::from_slice(collab).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a document.
    // --------------------------------------------------
    let alice_doc = WriteBuilder::new()
        .data(Data::from(b"A document".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://anyone-collaborate-protocol.xyz".to_string(),
            protocol_path: "doc".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_doc.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob updates Alice's document.
    // --------------------------------------------------
    let alice_doc = WriteBuilder::from(alice_doc)
        .data(Data::from(b"An update".to_vec()))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, alice_doc, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to create a new document.
    // --------------------------------------------------
    let bob_doc = WriteBuilder::new()
        .data(Data::from(b"A document".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://anyone-collaborate-protocol.xyz".to_string(),
            protocol_path: "doc".to_string(),
        })
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, bob_doc, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should allow creating records using an ancestor recipient rule.
#[tokio::test]
async fn ancestor_create() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let issuer_keyring = provider.keyring(ISSUER_DID).expect("should get VC issuer's keyring");

    // --------------------------------------------------
    // Alice configures a credential issuance protocol.
    // --------------------------------------------------
    let issuance = include_bytes!("../crates/dwn-test/protocols/credential-issuance.json");
    let definition: Definition = serde_json::from_slice(issuance).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a credential application to her web node to simulate a
    // credential application being sent to a VC issuer.
    // --------------------------------------------------
    let application = WriteBuilder::new()
        .data(Data::from(b"credential application data".to_vec()))
        .recipient(ISSUER_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication".to_string(),
        })
        .schema("https://identity.foundation/credential-manifest/schemas/credential-application")
        .data_format("application/json")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, application.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // The VC Issuer responds to Alice's request.
    // --------------------------------------------------
    let response = WriteBuilder::new()
        .data(Data::from(b"credential response data".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication/credentialResponse".to_string(),
        })
        .parent_context_id(application.context_id.unwrap())
        .schema("https://identity.foundation/credential-manifest/schemas/credential-response")
        .data_format("application/json")
        .sign(&issuer_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, response.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify VC Issuer's response was created.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&response.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"credential response data"))
    );
}

// Should allow creating records using an ancestor recipient rule.
#[tokio::test]
async fn ancestor_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a recipient protocol.
    // --------------------------------------------------
    let recipient = include_bytes!("../crates/dwn-test/protocols/recipient-can.json");
    let definition: Definition = serde_json::from_slice(recipient).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a post with Bob as the recipient.
    // --------------------------------------------------
    let alice_post = WriteBuilder::new()
        .data(Data::from(b"Hello Bob".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://recipient-can-protocol.xyz".to_string(),
            protocol_path: "post".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_post.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a post tag.
    // --------------------------------------------------
    let alice_tag = WriteBuilder::new()
        .data(Data::from(b"tag my post".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://recipient-can-protocol.xyz".to_string(),
            protocol_path: "post/tag".to_string(),
        })
        .parent_context_id(alice_post.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_tag.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob updates Alice's post.
    // --------------------------------------------------
    let bob_tag = WriteBuilder::from(alice_tag.clone())
        .data(Data::from(b"Bob's tag".to_vec()))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_tag.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to create a new post.
    // --------------------------------------------------
    let bob_tag = WriteBuilder::new()
        .data(Data::from(b"Bob's post".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://recipient-can-protocol.xyz".to_string(),
            protocol_path: "post/tag".to_string(),
        })
        .parent_context_id(alice_post.context_id.unwrap())
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, bob_tag.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should allow updates using a direct recipient rule.
#[tokio::test]
async fn direct_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures a recipient protocol.
    // --------------------------------------------------
    let recipient = include_bytes!("../crates/dwn-test/protocols/recipient-can.json");
    let definition: Definition = serde_json::from_slice(recipient).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a post with Bob as the recipient.
    // --------------------------------------------------
    let alice_post = WriteBuilder::new()
        .data(Data::from(b"Hello Bob".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://recipient-can-protocol.xyz".to_string(),
            protocol_path: "post".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_post.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Carol attempts (but fails) to update Alice's post.
    // --------------------------------------------------
    let carol_update = WriteBuilder::from(alice_post.clone())
        .data(Data::from(b"Carol's update".to_vec()))
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) =
        endpoint::handle(ALICE_DID, carol_update.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");

    // --------------------------------------------------
    // Bob updates Alice's post.
    // --------------------------------------------------
    let bob_update = WriteBuilder::from(alice_post.clone())
        .data(Data::from(b"Bob's update".to_vec()))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow author to block non-authors using an ancestor author rule.
#[tokio::test]
async fn block_non_author() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Bob configures the social media protocol.
    // --------------------------------------------------
    let social_media = include_bytes!("../crates/dwn-test/protocols/social-media.json");
    let definition: Definition = serde_json::from_slice(social_media).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&bob_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(BOB_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes an image to Bob's web node.
    // --------------------------------------------------
    let alice_image = WriteBuilder::new()
        .data(Data::from(b"cafe-aesthetic.jpg".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://social-media.xyz".to_string(),
            protocol_path: "image".to_string(),
        })
        .schema("imageSchema")
        .data_format("image/jpeg")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(BOB_DID, alice_image.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Carol attempts (but fails) to add a caption to Alice's image.
    // --------------------------------------------------
    let carol_caption = WriteBuilder::new()
        .data(Data::from(b"bad vibes! >:(".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://social-media.xyz".to_string(),
            protocol_path: "image/caption".to_string(),
        })
        .schema("captionSchema")
        .data_format("text/plain")
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) =
        endpoint::handle(BOB_DID, carol_caption.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");

    // --------------------------------------------------
    // Alice adds a caption to her image.
    // --------------------------------------------------
    let alice_caption = WriteBuilder::new()
        .data(Data::from(b"coffee and work vibes!".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://social-media.xyz".to_string(),
            protocol_path: "image/caption".to_string(),
        })
        .schema("captionSchema")
        .parent_context_id(alice_image.context_id.unwrap())
        .data_format("text/plain")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(BOB_DID, alice_caption.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify Alice was able to add her caption.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&alice_caption.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create query");
    let reply = endpoint::handle(BOB_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"coffee and work vibes!"))
    );
}

// Should allow author to update using an ancestor author rule.
#[tokio::test]
async fn ancestor_author_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the author-can protocol.
    // --------------------------------------------------
    let author_can = include_bytes!("../crates/dwn-test/protocols/author-can.json");
    let definition: Definition = serde_json::from_slice(author_can).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob creates a post.
    // --------------------------------------------------
    let bob_post = WriteBuilder::new()
        .data(Data::from(b"Bob's post".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://author-can-protocol.xyz".to_string(),
            protocol_path: "post".to_string(),
        })
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_post.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice comments on Bob's post
    // --------------------------------------------------
    let alice_comment = WriteBuilder::new()
        .data(Data::from(b"Alice's comment".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://author-can-protocol.xyz".to_string(),
            protocol_path: "post/comment".to_string(),
        })
        .parent_context_id(bob_post.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_comment.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob updates Alice's comment
    // --------------------------------------------------
    let bob_update = WriteBuilder::from(alice_comment)
        .data(Data::from(b"Update to Alice's comment".to_vec()))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to create a new comment on his post.
    // --------------------------------------------------
    let bob_post = WriteBuilder::new()
        .data(Data::from(b"Bob's comment".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://author-can-protocol.xyz".to_string(),
            protocol_path: "post/comment".to_string(),
        })
        .parent_context_id(bob_post.context_id.unwrap())
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, bob_post.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should allow a role record with recipient to be created and updated.
#[tokio::test]
async fn update_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice updates Bob's friend role record.
    // --------------------------------------------------
    let update = WriteBuilder::from(bob_friend)
        .data(Data::from(b"Bob is still my friend".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should reject a role record when no recipient is defined.
#[tokio::test]
async fn no_role_recipient() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice attempts (and fails) to add a role record with no recipient.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await
    else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "role record is missing recipient");
}

// Should allow a role record to be created for the same recipient after their
// previous record has been deleted.
#[tokio::test]
async fn recreate_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice removes Bob as a friend.
    // --------------------------------------------------
    let delete = DeleteBuilder::new()
        .record_id(&bob_friend.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");
    let reply = endpoint::handle(ALICE_DID, delete, &provider).await.expect("should delete");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend again.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend again".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow records to be created and updated using a context role.
#[tokio::test]
async fn context_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice updates Bob's role.
    // --------------------------------------------------
    let update_bob = WriteBuilder::from(bob_thread)
        .data(Data::from(b"Update Bob".to_vec()))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, update_bob.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow the same role to be created under different contexts.
#[tokio::test]
async fn context_roles() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread1 = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread1 = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread1.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts another thread.
    // --------------------------------------------------
    let thread2 = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the second thread.
    // --------------------------------------------------
    let bob_thread2 = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread2.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should reject attempts to create a duplicate role under same context.
#[tokio::test]
async fn duplicate_context_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice attempts (and fails) to add Bob to the thread again.
    // --------------------------------------------------
    let bob_thread2 = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) =
        endpoint::handle(ALICE_DID, bob_thread2.clone(), &provider).await
    else {
        panic!("should be BadRequest");
    };
    assert_eq!(e, "recipient already has this role record");
}

// Should allow a context role record to be created for the same recipient
// after their previous record has been deleted.
#[tokio::test]
async fn recreate_context_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice removes Bob from the thread.
    // --------------------------------------------------
    let delete = DeleteBuilder::new()
        .record_id(&bob_thread.record_id)
        .build(&alice_keyring)
        .await
        .expect("should create delete");
    let reply = endpoint::handle(ALICE_DID, delete, &provider).await.expect("should delete");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice re-adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread2 = WriteBuilder::new()
        .data(Data::from(b"Bob can rejoin my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow a creating records using role-based permissions.
#[tokio::test]
async fn role_can_create() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "friend".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob write a chat record.
    // --------------------------------------------------
    let bob_chat = WriteBuilder::new()
        .data(Data::from(b"Bob is Alice's friend".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
        })
        .protocol_role("friend")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow a updating records using role-based permissions.
#[tokio::test]
async fn role_can_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "admin".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a chat record.
    // --------------------------------------------------
    let alice_chat = WriteBuilder::new()
        .data(Data::from(b"Bob is Alice's friend".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob uses his 'admin' role to update the chat thread.
    // --------------------------------------------------
    let bob_update = WriteBuilder::from(alice_chat)
        .data(Data::from(b"I'm more than a friend".to_vec()))
        .protocol_role("admin")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should reject record creation if the recipient has not been assigned the
// protocol role.
#[tokio::test]
async fn invalid_protocol_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob as a friend.
    // --------------------------------------------------
    let bob_friend = WriteBuilder::new()
        .data(Data::from(b"Bob is my friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "admin".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_friend.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a chat record.
    // --------------------------------------------------
    let alice_chat = WriteBuilder::new()
        .data(Data::from(b"Bob is Alice's friend".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to use the 'chat' role because it does not exist.
    // --------------------------------------------------
    let bob_chat = WriteBuilder::new()
        .data(Data::from(b"I'm more than a friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
        })
        .protocol_role("chat")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, bob_chat.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "protocol path does not match role record type");
}

// Should reject record creation if the author has not been assigned the
// protocol role being used.
#[tokio::test]
async fn unassigned_protocol_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let friend_role = include_bytes!("../crates/dwn-test/protocols/friend-role.json");
    let definition: Definition = serde_json::from_slice(friend_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to use the 'friend' role because it has not
    // been assigned to him.
    // --------------------------------------------------
    let bob_chat = WriteBuilder::new()
        .data(Data::from(b"I'm more than a friend".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://friend-role.xyz".to_string(),
            protocol_path: "chat".to_string(),
        })
        .protocol_role("friend")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, bob_chat.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "unable to find record for role");
}

// Should allow record creation for authorized context role.
#[tokio::test]
async fn create_protocol_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob write a chat record.
    // --------------------------------------------------
    let bob_chat = WriteBuilder::new()
        .data(Data::from(b"Bob is Alice's friend".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/chat".to_string(),
        })
        .protocol_role("thread/participant")
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should allow record updates for authorized context role.
#[tokio::test]
async fn update_protocol_role() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the friend-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread = WriteBuilder::new()
        .data(Data::from(b"My new thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/admin".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice write a chat record.
    // --------------------------------------------------
    let alice_chat = WriteBuilder::new()
        .data(Data::from(b"Hello Bob".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/chat".to_string(),
        })
        .parent_context_id(thread.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, alice_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob write a chat record.
    // --------------------------------------------------
    let bob_chat = WriteBuilder::from(alice_chat)
        .data(Data::from(b"Hello wonderful Bob".to_vec()))
        .protocol_role("thread/admin")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_chat.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);
}

// Should reject creation of records when no access has been granted to the
// protocol role path.
#[tokio::test]
async fn forbidden_role_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice starts a new thread.
    // --------------------------------------------------
    let thread1 = WriteBuilder::new()
        .data(Data::from(b"Thread one".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread1.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice adds Bob to the thread.
    // --------------------------------------------------
    let bob_thread = WriteBuilder::new()
        .data(Data::from(b"Bob can join my thread".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/participant".to_string(),
        })
        .parent_context_id(thread1.context_id.as_ref().unwrap())
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_thread.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice creates a second thread.
    // --------------------------------------------------
    let thread2 = WriteBuilder::new()
        .data(Data::from(b"Thread two".to_vec()))
        .recipient(BOB_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, thread2.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to write a chat record to the second thread.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Hello Alice".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread/chat".to_string(),
        })
        .parent_context_id(thread2.context_id.as_ref().unwrap())
        .protocol_role("thread/participant")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, chat.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "unable to find record for role");
}

// Should reject creation of records using an invalid protocol path.
#[tokio::test]
async fn invalid_role_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the thread-role protocol.
    // --------------------------------------------------
    let thread_role = include_bytes!("../crates/dwn-test/protocols/thread-role.json");
    let definition: Definition = serde_json::from_slice(thread_role).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob attempts (and fails) to use a fake protocol role.
    // --------------------------------------------------
    let chat = WriteBuilder::new()
        .data(Data::from(b"Hello Alice".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://thread-role.xyz".to_string(),
            protocol_path: "thread".to_string(),
        })
        .protocol_role("not-a-real-path")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, chat.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "no rule set defined for invoked role");
}

// Should allow record updates by the initial author.
#[tokio::test]
async fn initial_author_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the message protocol.
    // --------------------------------------------------
    let message = include_bytes!("../crates/dwn-test/protocols/message.json");
    let definition: Definition = serde_json::from_slice(message).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes a message.
    // --------------------------------------------------
    let bob_msg = WriteBuilder::new()
        .data(Data::from(b"Hello from Bob".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://message-protocol.xyz".to_string(),
            protocol_path: "message".to_string(),
        })
        .schema("http://message.me")
        .data_format("text/plain")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_msg.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record was created.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&bob_msg.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, query.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"Hello from Bob"))
    );

    // --------------------------------------------------
    // Bob updates his message.
    // --------------------------------------------------
    let update = WriteBuilder::from(bob_msg)
        .data(Data::from(b"Hello, this is your friend Bob".to_vec()))
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply = endpoint::handle(ALICE_DID, update.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the update.
    // --------------------------------------------------
    let reply = endpoint::handle(ALICE_DID, query, &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"Hello, this is your friend Bob"))
    );
}

// Should prevent record updates by another author who does not have permission.
#[tokio::test]
async fn no_author_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");
    let carol_keyring = provider.keyring(CAROL_DID).expect("should get Carol's keyring");

    // --------------------------------------------------
    // Alice configures the message protocol.
    // --------------------------------------------------
    let message = include_bytes!("../crates/dwn-test/protocols/message.json");
    let definition: Definition = serde_json::from_slice(message).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes a message.
    // --------------------------------------------------
    let bob_msg = WriteBuilder::new()
        .data(Data::from(b"Hello from Bob".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://message-protocol.xyz".to_string(),
            protocol_path: "message".to_string(),
        })
        .schema("http://message.me")
        .data_format("text/plain")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_msg.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record was created.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&bob_msg.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, query.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"Hello from Bob"))
    );

    // --------------------------------------------------
    // Carol attempts (but fails) to update Bob's message.
    // --------------------------------------------------
    let update = WriteBuilder::new()
        .data(Data::from(b"Hello, this is your friend Carol".to_vec()))
        .record_id(bob_msg.record_id)
        .protocol(WriteProtocol {
            protocol: "http://message-protocol.xyz".to_string(),
            protocol_path: "message".to_string(),
        })
        .schema("http://message.me")
        .data_format("text/plain")
        .sign(&carol_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, update.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should prevent updates to the immutable `recipient` property.
#[tokio::test]
async fn no_recipient_update() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let bob_keyring = provider.keyring(BOB_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures the message protocol.
    // --------------------------------------------------
    let message = include_bytes!("../crates/dwn-test/protocols/message.json");
    let definition: Definition = serde_json::from_slice(message).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Bob writes a message.
    // --------------------------------------------------
    let bob_msg = WriteBuilder::new()
        .data(Data::from(b"Hello from Bob".to_vec()))
        .protocol(WriteProtocol {
            protocol: "http://message-protocol.xyz".to_string(),
            protocol_path: "message".to_string(),
        })
        .schema("http://message.me")
        .data_format("text/plain")
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, bob_msg.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Verify the record was created.
    // --------------------------------------------------
    let query = QueryBuilder::new()
        .filter(RecordsFilter::new().record_id(&bob_msg.record_id))
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create read");
    let reply = endpoint::handle(ALICE_DID, query.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::OK);

    let body = reply.body.expect("should have body");
    let entries = body.entries.expect("should have entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].write.encoded_data,
        Some(Base64UrlUnpadded::encode_string(b"Hello from Bob"))
    );

    // --------------------------------------------------
    // Bob attempts (but fails) to update the message's recipient.
    // --------------------------------------------------
    let update = WriteBuilder::new()
        .data(Data::from(b"Hello, this is your friend Carol".to_vec()))
        .record_id(bob_msg.record_id)
        .protocol(WriteProtocol {
            protocol: "http://message-protocol.xyz".to_string(),
            protocol_path: "message".to_string(),
        })
        .schema("http://message.me")
        .data_format("text/plain")
        .recipient(CAROL_DID)
        .sign(&bob_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::BadRequest(e)) = endpoint::handle(ALICE_DID, update.clone(), &provider).await
    else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "immutable properties do not match");
}

// Should prevent unauthorized record creation using a `recipient` rule.
#[tokio::test]
async fn unauthorized_create() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");
    let fake_keyring = provider.keyring(FAKE_DID).expect("should get Bob's keyring");

    // --------------------------------------------------
    // Alice configures a credential issuance protocol.
    // --------------------------------------------------
    let issuance = include_bytes!("../crates/dwn-test/protocols/credential-issuance.json");
    let definition: Definition = serde_json::from_slice(issuance).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a credential application to her web node to simulate a
    // credential application being sent to a VC issuer.
    // --------------------------------------------------
    let application = WriteBuilder::new()
        .data(Data::from(b"credential application data".to_vec()))
        .recipient(ISSUER_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication".to_string(),
        })
        .schema("https://identity.foundation/credential-manifest/schemas/credential-application")
        .data_format("application/json")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let reply =
        endpoint::handle(ALICE_DID, application.clone(), &provider).await.expect("should write");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // A fake VC Issuer responds to Alice's request.
    // --------------------------------------------------
    let response = WriteBuilder::new()
        .data(Data::from(b"credential response data".to_vec()))
        .recipient(ALICE_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication/credentialResponse".to_string(),
        })
        .parent_context_id(application.context_id.unwrap())
        .schema("https://identity.foundation/credential-manifest/schemas/credential-response")
        .data_format("application/json")
        .sign(&fake_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, response, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "action not permitted");
}

// Should prevent record creation when protocol cannot be found.
#[tokio::test]
async fn no_protocol_definition() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice writes a credential application to her web node without the
    // credential issuance protocol installed.
    // --------------------------------------------------
    let application = WriteBuilder::new()
        .data(Data::from(b"credential application data".to_vec()))
        .recipient(ISSUER_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication".to_string(),
        })
        .schema("https://identity.foundation/credential-manifest/schemas/credential-application")
        .data_format("application/json")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, application, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "unable to find protocol definition");
}

// Should prevent record creation when schema is invalid.
#[tokio::test]
async fn invalid_schema() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures a credential issuance protocol.
    // --------------------------------------------------
    let issuance = include_bytes!("../crates/dwn-test/protocols/credential-issuance.json");
    let definition: Definition = serde_json::from_slice(issuance).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a credential application using an invalid schema.
    // --------------------------------------------------
    let application = WriteBuilder::new()
        .data(Data::from(b"credential application data".to_vec()))
        .recipient(ISSUER_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "credentialApplication".to_string(),
        })
        .schema("unexpected-schema")
        .data_format("application/json")
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, application, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "invalid schema");
}

// Should prevent record creation when protocol path is invalid.
#[tokio::test]
async fn invalid_protocol_path() {
    let provider = ProviderImpl::new().await.expect("should create provider");
    let alice_keyring = provider.keyring(ALICE_DID).expect("should get Alice's keyring");

    // --------------------------------------------------
    // Alice configures a credential issuance protocol.
    // --------------------------------------------------
    let issuance = include_bytes!("../crates/dwn-test/protocols/credential-issuance.json");
    let definition: Definition = serde_json::from_slice(issuance).expect("should deserialize");
    let configure = ConfigureBuilder::new()
        .definition(definition.clone())
        .build(&alice_keyring)
        .await
        .expect("should build");
    let reply =
        endpoint::handle(ALICE_DID, configure, &provider).await.expect("should configure protocol");
    assert_eq!(reply.status.code, StatusCode::ACCEPTED);

    // --------------------------------------------------
    // Alice writes a credential application using an invalid schema.
    // --------------------------------------------------
    let application = WriteBuilder::new()
        .data(Data::from(b"credential application data".to_vec()))
        .recipient(ISSUER_DID)
        .protocol(WriteProtocol {
            protocol: "http://credential-issuance-protocol.xyz".to_string(),
            protocol_path: "invalidType".to_string(),
        })
        .sign(&alice_keyring)
        .build()
        .await
        .expect("should create write");
    let Err(Error::Forbidden(e)) = endpoint::handle(ALICE_DID, application, &provider).await else {
        panic!("should be Forbidden");
    };
    assert_eq!(e, "invalid protocol path");
}
