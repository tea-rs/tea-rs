use std::str::FromStr;
use std::time::Duration;

use tea::RuntimeEventSink;
use tea_kernel::KernelEventSink;
use tea_protocol::{
    AgentEvent, EventEnvelope, EventId, ProtocolMetadata, ProtocolTimestamp, RunId, SessionId,
    SessionSequence,
};

fn envelope(session: &str, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EventId::from_str(&format!("0195a0b1-0000-7000-8000-{sequence:012}")).unwrap(),
        SessionId::from_str(session).unwrap(),
        Some(RunId::from_str("0195a0b1-0001-7000-8000-000000000001").unwrap()),
        None,
        SessionSequence::new(sequence),
        ProtocolTimestamp::from_str("2026-07-23T09:30:12.123Z").unwrap(),
        ProtocolMetadata::default(),
        AgentEvent::RunStarted {},
    )
    .unwrap()
}

#[tokio::test]
async fn subscriber_receives_events_in_sequence() {
    let sink = RuntimeEventSink::new();
    let session = "0195a0b1-5e3a-7d72-a902-c4e85d828bf1";
    let mut receiver = sink
        .subscribe(SessionId::from_str(session).unwrap())
        .unwrap();
    for sequence in 1..=3 {
        sink.emit(envelope(session, sequence)).await.unwrap();
    }
    let mut sequences = Vec::new();
    for _ in 0..3 {
        sequences.push(receiver.recv().await.unwrap().sequence().get());
    }
    assert_eq!(sequences, [1, 2, 3]);
    assert_eq!(
        sink.last_sequence(SessionId::from_str(session).unwrap()),
        Some(SessionSequence::new(3))
    );
}

#[tokio::test]
async fn full_channel_applies_backpressure_until_drained() {
    let sink = RuntimeEventSink::with_capacity(2);
    let session = "0195a0b1-5e3a-7d72-a902-c4e85d828bf2";
    let mut receiver = sink
        .subscribe(SessionId::from_str(session).unwrap())
        .unwrap();
    // Fill the channel exactly to capacity.
    sink.emit(envelope(session, 1)).await.unwrap();
    sink.emit(envelope(session, 2)).await.unwrap();
    // A third emit must block until the receiver drains.
    let blocked =
        tokio::time::timeout(Duration::from_millis(50), sink.emit(envelope(session, 3))).await;
    assert!(blocked.is_err(), "emit should block on a full channel");
    // Drain one slot and the blocked emit completes.
    assert_eq!(receiver.recv().await.unwrap().sequence().get(), 1);
    tokio::time::timeout(Duration::from_millis(200), sink.emit(envelope(session, 3)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receiver.recv().await.unwrap().sequence().get(), 2);
    assert_eq!(receiver.recv().await.unwrap().sequence().get(), 3);
}

#[tokio::test]
async fn dropped_receiver_is_removed_without_failing_remaining() {
    let sink = RuntimeEventSink::new();
    let session = "0195a0b1-5e3a-7d72-a902-c4e85d828bf3";
    let first = sink
        .subscribe(SessionId::from_str(session).unwrap())
        .unwrap();
    let mut survivor = sink
        .subscribe(SessionId::from_str(session).unwrap())
        .unwrap();
    drop(first);
    // Emit fills the first channel then is pruned; the survivor still receives.
    sink.emit(envelope(session, 1)).await.unwrap();
    assert_eq!(survivor.recv().await.unwrap().sequence().get(), 1);
    // A second emit completes without error after pruning.
    sink.emit(envelope(session, 2)).await.unwrap();
    assert_eq!(survivor.recv().await.unwrap().sequence().get(), 2);
}

#[tokio::test]
async fn session_without_subscribers_accepts_without_backpressure() {
    let sink = RuntimeEventSink::new();
    let session = "0195a0b1-5e3a-7d72-a902-c4e85d828bf4";
    sink.emit(envelope(session, 1)).await.unwrap();
    assert_eq!(
        sink.last_sequence(SessionId::from_str(session).unwrap()),
        Some(SessionSequence::new(1))
    );
}
