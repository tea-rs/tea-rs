use std::str::FromStr;

use tea_protocol::{
    ModelId, ProfileId, ProtocolMetadata, ProtocolTimestamp, RecordEnvelope, RecordId, SessionId,
    SessionRecord, SessionSequence,
};
use tea_session::SessionReducer;

const RECORD_COUNT: u64 = 1024;
const SESSION_ID: &str = "0195a0b1-7000-7000-8000-0aa7aa000001";
const TIMESTAMP: &str = "2026-07-25T12:00:00.000Z";

fn record_id(sequence: u64) -> RecordId {
    RecordId::from_str(&format!("0195a0b1-7000-7000-8000-{sequence:012x}")).unwrap()
}

fn envelope(sequence: u64, record: SessionRecord) -> RecordEnvelope {
    RecordEnvelope::new(
        record_id(sequence),
        SessionId::from_str(SESSION_ID).unwrap(),
        SessionSequence::new(sequence),
        ProtocolTimestamp::from_str(TIMESTAMP).unwrap(),
        None,
        None,
        None,
        ProtocolMetadata::default(),
        record,
    )
    .unwrap()
}

fn generated_records() -> Vec<RecordEnvelope> {
    let mut records = Vec::with_capacity(usize::try_from(RECORD_COUNT).unwrap());
    records.push(envelope(
        0,
        SessionRecord::SessionCreated {
            profile_id: ProfileId::from_str("minimal-assistant").unwrap(),
            metadata: ProtocolMetadata::default(),
        },
    ));
    for sequence in 1..RECORD_COUNT {
        records.push(envelope(
            sequence,
            SessionRecord::ConfigurationChanged {
                model: Some(tea_protocol::ModelRef::new(
                    "test".parse().unwrap(),
                    ModelId::from_str(&format!("test/model-{sequence}")).unwrap(),
                )),
                profile_id: None,
                reasoning_effort: None,
            },
        ));
    }
    records
}

#[test]
fn generated_replay_capacity_is_deterministic_at_1024_records() {
    let records = generated_records();
    let expected = SessionReducer::replay(records.clone()).unwrap();
    let mut incremental = SessionReducer::new();
    for record in &records {
        incremental.apply(record).unwrap();
    }

    assert_eq!(records.len(), usize::try_from(RECORD_COUNT).unwrap());
    assert_eq!(
        expected.tail_sequence(),
        SessionSequence::new(RECORD_COUNT - 1)
    );
    assert_eq!(incremental.state(), Some(&expected));
}
