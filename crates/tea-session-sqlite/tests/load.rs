//! Fault-injection and long-session load coverage for the `SQLite` store.

use std::str::FromStr;
use std::time::Duration;

use tea_protocol::{
    CanonicalMessage, ContentBlock, MessageId, ProfileId, ProtocolMetadata, ProtocolTimestamp,
    RecordEnvelope, RecordId, SessionId, SessionRecord, SessionSequence,
};
use tea_session::{AppendTransaction, SessionStore, SessionStoreErrorCode};
use tea_session_sqlite::SqliteSessionStore;

const NOW: &str = "2026-07-23T09:30:12.125Z";

fn session_id() -> SessionId {
    SessionId::from_str("0195a0b1-5e3a-7d72-a902-c4e85d828bf1").unwrap()
}
fn timestamp() -> ProtocolTimestamp {
    ProtocolTimestamp::from_str(NOW).unwrap()
}
fn envelope(seq: u64, rid: &str, record: SessionRecord) -> RecordEnvelope {
    RecordEnvelope::new(
        RecordId::from_str(rid).unwrap(),
        session_id(),
        SessionSequence::new(seq),
        timestamp(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        record,
    )
    .unwrap()
}
fn user_message(seq: u64, n: u64) -> RecordEnvelope {
    envelope(
        seq,
        &format!("0195a0b1-5e52-79e1-8f4a-{n:012x}"),
        SessionRecord::MessageCommitted {
            message: CanonicalMessage::user(
                MessageId::from_str(&format!("0195a0b1-5e6a-74b2-8c25-{n:012x}")).unwrap(),
                vec![ContentBlock::text(format!("turn {n}")).unwrap()],
                timestamp(),
            )
            .unwrap(),
        },
    )
}

#[tokio::test]
async fn concurrent_appends_with_stale_sequence_conflict() {
    let store = std::sync::Arc::new(SqliteSessionStore::in_memory().unwrap());
    store
        .append(AppendTransaction::new(
            session_id(),
            None,
            vec![envelope(
                0,
                "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
                SessionRecord::SessionCreated {
                    profile_id: ProfileId::from_str("coding").unwrap(),
                    metadata: ProtocolMetadata::default(),
                },
            )],
        ))
        .await
        .unwrap();
    // Two concurrent appends both expecting tail 0: only one can win.
    let left = std::sync::Arc::clone(&store);
    let right = std::sync::Arc::clone(&store);
    let left_handle = tokio::spawn(async move {
        left.append(AppendTransaction::new(
            session_id(),
            Some(SessionSequence::new(0)),
            vec![user_message(1, 1)],
        ))
        .await
    });
    let right_handle = tokio::spawn(async move {
        right
            .append(AppendTransaction::new(
                session_id(),
                Some(SessionSequence::new(0)),
                vec![user_message(1, 2)],
            ))
            .await
    });
    let left_result = left_handle.await.unwrap();
    let right_result = right_handle.await.unwrap();
    // Exactly one append must succeed; the other receives a SequenceConflict.
    let successes = [left_result.is_ok(), right_result.is_ok()]
        .iter()
        .filter(|&&ok| ok)
        .count();
    assert_eq!(successes, 1, "exactly one concurrent append must win");
    let [left_result, right_result] = [left_result, right_result];
    let conflict = [&left_result, &right_result]
        .into_iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert_eq!(conflict.code(), SessionStoreErrorCode::SequenceConflict);
}

#[tokio::test]
async fn long_session_round_trip_through_many_turns() {
    let store = std::sync::Arc::new(SqliteSessionStore::in_memory().unwrap());
    store
        .append(AppendTransaction::new(
            session_id(),
            None,
            vec![
                envelope(
                    0,
                    "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
                    SessionRecord::SessionCreated {
                        profile_id: ProfileId::from_str("coding").unwrap(),
                        metadata: ProtocolMetadata::default(),
                    },
                ),
                envelope(
                    1,
                    "0195a0b1-5e51-79e1-8f4a-0aa7aa000002",
                    SessionRecord::ConfigurationChanged {
                        model: Some(tea_protocol::ModelRef::new(
                            "fake".parse().unwrap(),
                            "fake/model".parse().unwrap(),
                        )),
                        profile_id: None,
                        reasoning_effort: None,
                    },
                ),
            ],
        ))
        .await
        .unwrap();
    let turns = 50u64;
    let mut tail = SessionSequence::new(1);
    for n in 1..=turns {
        let seq = 1 + n;
        let record = user_message(seq, n);
        let outcome = store
            .append(AppendTransaction::new(
                session_id(),
                Some(tail),
                vec![record],
            ))
            .await
            .unwrap();
        tail = outcome.current_sequence();
    }
    let snapshot = store.load(session_id()).await.unwrap();
    assert_eq!(
        snapshot.state().messages().len(),
        usize::try_from(turns).unwrap()
    );
    assert_eq!(
        snapshot.state().tail_sequence(),
        SessionSequence::new(1 + turns),
    );
}

#[tokio::test]
async fn append_inserts_only_delta_rows_and_preserves_existing_rowids() {
    let path = format!("/tmp/tea-incremental-{}.sqlite", std::process::id());
    let _ = std::fs::remove_file(&path);
    let store = SqliteSessionStore::open(&path).unwrap();
    store
        .append(AppendTransaction::new(
            session_id(),
            None,
            vec![envelope(
                0,
                "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
                SessionRecord::SessionCreated {
                    profile_id: ProfileId::from_str("coding").unwrap(),
                    metadata: ProtocolMetadata::default(),
                },
            )],
        ))
        .await
        .unwrap();
    let before = stored_rows(&path);

    store
        .append(AppendTransaction::new(
            session_id(),
            Some(SessionSequence::new(0)),
            vec![user_message(1, 1), user_message(2, 2), user_message(3, 3)],
        ))
        .await
        .unwrap();
    let after = stored_rows(&path);

    assert_eq!(&after[..before.len()], before.as_slice());
    assert_eq!(after.len(), before.len() + 3);
    assert_eq!(
        after
            .iter()
            .map(|(_, sequence, _, _)| *sequence)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[tokio::test(flavor = "current_thread")]
async fn sqlite_lock_wait_does_not_block_tokio_timer() {
    let path = format!("/tmp/tea-worker-nonblocking-{}.sqlite", std::process::id());
    let _ = std::fs::remove_file(&path);
    let store = std::sync::Arc::new(SqliteSessionStore::open(&path).unwrap());
    store
        .append(AppendTransaction::new(
            session_id(),
            None,
            vec![envelope(
                0,
                "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
                SessionRecord::SessionCreated {
                    profile_id: ProfileId::from_str("coding").unwrap(),
                    metadata: ProtocolMetadata::default(),
                },
            )],
        ))
        .await
        .unwrap();

    let blocker = rusqlite::Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let writer = std::sync::Arc::clone(&store);
    let append = tokio::spawn(async move {
        writer
            .append(AppendTransaction::new(
                session_id(),
                Some(SessionSequence::new(0)),
                vec![user_message(1, 1)],
            ))
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !append.is_finished(),
        "SQLite writer should be waiting on the lock"
    );

    tokio::time::timeout(
        Duration::from_millis(250),
        tokio::time::sleep(Duration::from_millis(25)),
    )
    .await
    .expect("Tokio timer must advance while the SQLite worker is blocked");
    blocker.execute_batch("ROLLBACK").unwrap();
    append.await.unwrap().unwrap();
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
#[ignore = "manual 1K/10K SQLite latency and projection-clone regression benchmark"]
async fn benchmark_1k_10k_append_load_reopen_and_write_rows() {
    for record_count in [1_000_u64, 10_000] {
        let path = format!(
            "/tmp/tea-benchmark-{record_count}-{}.sqlite",
            std::process::id()
        );
        let _ = std::fs::remove_file(&path);
        let store = SqliteSessionStore::open(&path).unwrap();
        let mut records = Vec::with_capacity(usize::try_from(record_count).unwrap() + 1);
        records.push(envelope(
            0,
            "0195a0b1-5e50-79e1-8f4a-0aa7aa000001",
            SessionRecord::SessionCreated {
                profile_id: ProfileId::from_str("coding").unwrap(),
                metadata: ProtocolMetadata::default(),
            },
        ));
        records.extend((1..=record_count).map(|sequence| user_message(sequence, sequence)));

        let append_started = std::time::Instant::now();
        store
            .append(AppendTransaction::new(session_id(), None, records))
            .await
            .unwrap();
        let batch_append = append_started.elapsed();
        let rows_after_batch = stored_rows(&path).len();
        assert_eq!(rows_after_batch, usize::try_from(record_count).unwrap() + 1);

        let tail_started = std::time::Instant::now();
        store
            .append(AppendTransaction::new(
                session_id(),
                Some(SessionSequence::new(record_count)),
                vec![user_message(record_count + 1, record_count + 1)],
            ))
            .await
            .unwrap();
        let tail_append = tail_started.elapsed();
        assert_eq!(stored_rows(&path).len(), rows_after_batch + 1);
        drop(store);

        let reopen_started = std::time::Instant::now();
        let reopened = SqliteSessionStore::open(&path).unwrap();
        let open = reopen_started.elapsed();
        let load_started = std::time::Instant::now();
        let snapshot = reopened.load(session_id()).await.unwrap();
        let load = load_started.elapsed();
        let clone_started = std::time::Instant::now();
        std::hint::black_box(snapshot.state().clone());
        let projection_clone = clone_started.elapsed();

        eprintln!(
            "records={record_count} batch_append={batch_append:?} tail_append={tail_append:?} \
             reopen={open:?} load={load:?} projection_clone={projection_clone:?} rows={}",
            snapshot.records().len()
        );
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}

fn stored_rows(path: &str) -> Vec<(i64, i64, String, String)> {
    let connection = rusqlite::Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT rowid, sequence, record_id, envelope FROM records ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
