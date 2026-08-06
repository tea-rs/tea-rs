use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{mpsc, oneshot};

use super::reader::MAX_RPC_FRAME_BYTES;

/// Maximum queued RPC output frames.
pub const RPC_WRITER_QUEUE_CAPACITY: usize = 32;
/// Maximum enqueue, write, flush, or shutdown latency.
pub const RPC_WRITER_DEADLINE: Duration = Duration::from_millis(500);

/// Bounded RPC output failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RpcWriteError {
    /// The serialized response was invalid or oversized.
    #[error("RPC output value is invalid")]
    InvalidValue,
    /// The output task or stream closed.
    #[error("RPC output is closed")]
    Closed,
    /// The client did not accept output within the deadline.
    #[error("RPC output deadline exceeded")]
    Deadline,
}

struct WriteRequest {
    line: Vec<u8>,
    acknowledgement: oneshot::Sender<Result<(), RpcWriteError>>,
}

/// Owned bounded LF writer with per-frame flush acknowledgements.
#[derive(Debug)]
pub struct RpcLineWriter {
    sender: Option<mpsc::Sender<WriteRequest>>,
    task: tokio::task::JoinHandle<Result<(), RpcWriteError>>,
    deadline: Duration,
}

impl RpcLineWriter {
    /// Starts one owned writer task.
    #[must_use]
    pub fn spawn<W>(output: W) -> Self
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::spawn_with_deadline(output, RPC_WRITER_DEADLINE)
    }

    /// Starts one writer with an explicit deadline for deterministic tests.
    #[must_use]
    pub fn spawn_with_deadline<W>(output: W, deadline: Duration) -> Self
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, receiver) = mpsc::channel(RPC_WRITER_QUEUE_CAPACITY);
        let task = tokio::spawn(writer_loop(output, receiver, deadline));
        Self {
            sender: Some(sender),
            task,
            deadline,
        }
    }

    /// Serializes, enqueues, writes, and flushes one compact JSON/LF frame.
    ///
    /// # Errors
    ///
    /// Returns an invalid-value, closed-output, or deadline error.
    pub async fn write<T: Serialize>(&self, value: &T) -> Result<(), RpcWriteError> {
        let line = encode_line(value)?;
        let (acknowledgement, received) = oneshot::channel();
        let request = WriteRequest {
            line,
            acknowledgement,
        };
        let sender = self.sender.as_ref().ok_or(RpcWriteError::Closed)?;
        tokio::time::timeout(self.deadline, sender.send(request))
            .await
            .map_err(|_| RpcWriteError::Deadline)?
            .map_err(|_| RpcWriteError::Closed)?;
        tokio::time::timeout(self.deadline, received)
            .await
            .map_err(|_| RpcWriteError::Deadline)?
            .map_err(|_| RpcWriteError::Closed)?
    }

    /// Closes the queue and awaits the owned writer task.
    ///
    /// # Errors
    ///
    /// Returns when draining or joining exceeds the deadline or output fails.
    pub async fn shutdown(mut self) -> Result<(), RpcWriteError> {
        self.sender.take();
        tokio::time::timeout(self.deadline, &mut self.task)
            .await
            .map_err(|_| {
                self.task.abort();
                RpcWriteError::Deadline
            })?
            .map_err(|_| RpcWriteError::Closed)?
    }
}

fn encode_line<T: Serialize>(value: &T) -> Result<Vec<u8>, RpcWriteError> {
    let mut line = serde_json::to_vec(value).map_err(|_| RpcWriteError::InvalidValue)?;
    if line.is_empty()
        || line.len() > MAX_RPC_FRAME_BYTES
        || line.contains(&b'\n')
        || line.contains(&b'\r')
    {
        return Err(RpcWriteError::InvalidValue);
    }
    line.push(b'\n');
    Ok(line)
}

async fn writer_loop<W: AsyncWrite + Unpin>(
    mut output: W,
    mut receiver: mpsc::Receiver<WriteRequest>,
    deadline: Duration,
) -> Result<(), RpcWriteError> {
    while let Some(request) = receiver.recv().await {
        let result = tokio::time::timeout(deadline, async {
            output
                .write_all(&request.line)
                .await
                .map_err(|_| RpcWriteError::Closed)?;
            output.flush().await.map_err(|_| RpcWriteError::Closed)
        })
        .await
        .map_err(|_| RpcWriteError::Deadline)
        .and_then(std::convert::identity);
        let failed = result.is_err();
        let _ = request.acknowledgement.send(result);
        if failed {
            return Err(result.expect_err("failed result checked above"));
        }
    }
    Ok(())
}
