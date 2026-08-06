use std::io::Write;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::oneshot;

/// Maximum compact JSON bytes in one line, excluding its LF delimiter.
pub const MAX_JSON_LINE_BYTES: usize = 1024 * 1024;
/// Bounded external writer queue capacity.
pub const JSON_WRITER_QUEUE_CAPACITY: usize = 32;
/// Maximum time one enqueue or blocking write acknowledgment may consume.
pub const JSON_WRITER_DEADLINE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonLineFailure {
    InvalidValue,
    Closed,
    Deadline,
}

struct WriteRequest {
    line: Vec<u8>,
    acknowledgement: oneshot::Sender<Result<(), JsonLineFailure>>,
}

/// Strict compact-JSON/LF writer isolated behind a bounded blocking queue.
#[derive(Debug, Clone)]
pub(crate) struct JsonLineWriter {
    sender: SyncSender<WriteRequest>,
    deadline: Duration,
}

impl JsonLineWriter {
    pub(crate) fn spawn(
        output: Box<dyn Write + Send>,
        deadline: Duration,
    ) -> Result<Self, JsonLineFailure> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(JSON_WRITER_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("tea-json-writer".to_owned())
            .spawn(move || writer_loop(output, &receiver))
            .map_err(|_| JsonLineFailure::Closed)?;
        Ok(Self { sender, deadline })
    }

    pub(crate) async fn write<T: Serialize>(&self, value: &T) -> Result<(), JsonLineFailure> {
        let line = encode_line(value)?;
        let (acknowledgement, received) = oneshot::channel();
        let request = WriteRequest {
            line,
            acknowledgement,
        };
        self.enqueue(request).await?;
        tokio::time::timeout(self.deadline, received)
            .await
            .map_err(|_| JsonLineFailure::Deadline)?
            .map_err(|_| JsonLineFailure::Closed)?
    }

    async fn enqueue(&self, mut request: WriteRequest) -> Result<(), JsonLineFailure> {
        let deadline = tokio::time::Instant::now() + self.deadline;
        loop {
            match self.sender.try_send(request) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(_)) => return Err(JsonLineFailure::Closed),
                Err(TrySendError::Full(returned)) => request = returned,
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(JsonLineFailure::Deadline);
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(5))).await;
        }
    }
}

fn encode_line<T: Serialize>(value: &T) -> Result<Vec<u8>, JsonLineFailure> {
    let mut line = serde_json::to_vec(value).map_err(|_| JsonLineFailure::InvalidValue)?;
    if line.is_empty()
        || line.len() > MAX_JSON_LINE_BYTES
        || line.contains(&b'\n')
        || line.contains(&b'\r')
    {
        return Err(JsonLineFailure::InvalidValue);
    }
    line.push(b'\n');
    Ok(line)
}

fn writer_loop(
    mut output: Box<dyn Write + Send>,
    receiver: &std::sync::mpsc::Receiver<WriteRequest>,
) {
    while let Ok(request) = receiver.recv() {
        let result = output
            .write_all(&request.line)
            .and_then(|()| output.flush())
            .map_err(|_| JsonLineFailure::Closed);
        let failed = result.is_err();
        let _ = request.acknowledgement.send(result);
        if failed {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone)]
    struct SharedOutput(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct BrokenOutput;
    impl Write for BrokenOutput {
        fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct SlowOutput;
    impl Write for SlowOutput {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_millis(100));
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn writes_one_compact_lf_delimited_value() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = JsonLineWriter::spawn(
            Box::new(SharedOutput(Arc::clone(&bytes))),
            Duration::from_secs(1),
        )
        .unwrap();
        writer
            .write(&serde_json::json!({"text":"a\n\u{2028}b"}))
            .await
            .unwrap();
        let output = bytes.lock().unwrap().clone();
        assert_eq!(output.last(), Some(&b'\n'));
        assert!(!output[..output.len() - 1].contains(&b'\n'));
        assert!(serde_json::from_slice::<serde_json::Value>(&output[..output.len() - 1]).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canonical_tool_progress_fixture_remains_one_line() {
        let event = serde_json::from_str::<tea_protocol::EventEnvelope>(include_str!(
            "../../tea-protocol/tests/fixtures/v1.0/event-tool-execution-progress.json"
        ))
        .unwrap();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = JsonLineWriter::spawn(
            Box::new(SharedOutput(Arc::clone(&bytes))),
            Duration::from_secs(1),
        )
        .unwrap();
        writer.write(&event).await.unwrap();
        let output = bytes.lock().unwrap().clone();
        let value =
            serde_json::from_slice::<serde_json::Value>(&output[..output.len() - 1]).unwrap();
        assert_eq!(value["type"], "tool_execution_progress");
        assert_eq!(value["payload"]["message"], "writing file");
        assert_eq!(output.last(), Some(&b'\n'));
        assert!(!output[..output.len() - 1].contains(&b'\n'));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_oversized_line_before_enqueue() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let writer = JsonLineWriter::spawn(
            Box::new(SharedOutput(Arc::clone(&bytes))),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(
            writer
                .write(&serde_json::json!({"value":"x".repeat(MAX_JSON_LINE_BYTES)}))
                .await,
            Err(JsonLineFailure::InvalidValue)
        );
        assert!(bytes.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn broken_and_slow_writers_fail_within_deadline() {
        let broken =
            JsonLineWriter::spawn(Box::new(BrokenOutput), Duration::from_millis(20)).unwrap();
        assert_eq!(
            broken.write(&serde_json::json!({"ok":true})).await,
            Err(JsonLineFailure::Closed)
        );
        let slow = JsonLineWriter::spawn(Box::new(SlowOutput), Duration::from_millis(20)).unwrap();
        assert_eq!(
            slow.write(&serde_json::json!({"ok":true})).await,
            Err(JsonLineFailure::Deadline)
        );
    }
}
