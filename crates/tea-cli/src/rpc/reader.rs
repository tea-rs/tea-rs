use std::io;

use tokio::io::{AsyncRead, AsyncReadExt as _};

/// Maximum bytes accepted before one LF delimiter.
pub const MAX_RPC_FRAME_BYTES: usize = 1024 * 1024;
const READ_CHUNK_BYTES: usize = 8192;

/// Terminal strict-framing or input failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RpcReadError {
    /// A frame exceeded the configured bound before its LF delimiter.
    #[error("RPC frame exceeds the size limit")]
    Oversize,
    /// EOF arrived after bytes that were not terminated by LF.
    #[error("RPC input ended inside a frame")]
    Unterminated,
    /// The underlying input failed.
    #[error("RPC input is unavailable")]
    Io,
}

/// Strict byte-oriented LF frame reader with a fixed memory bound.
#[derive(Debug)]
pub struct RpcFrameReader<R> {
    input: R,
    buffered: Vec<u8>,
    eof: bool,
}

impl<R: AsyncRead + Unpin> RpcFrameReader<R> {
    /// Wraps one async byte stream.
    #[must_use]
    pub const fn new(input: R) -> Self {
        Self {
            input,
            buffered: Vec::new(),
            eof: false,
        }
    }

    /// Reads one complete frame, excluding LF and an optional preceding CR.
    ///
    /// # Errors
    ///
    /// Returns a terminal error for oversized, unterminated, or failed input.
    pub async fn read_frame(&mut self) -> Result<Option<Vec<u8>>, RpcReadError> {
        loop {
            if let Some(index) = self.buffered.iter().position(|byte| *byte == b'\n') {
                if index > MAX_RPC_FRAME_BYTES {
                    return Err(RpcReadError::Oversize);
                }
                let mut frame = self.buffered.drain(..=index).collect::<Vec<_>>();
                frame.pop();
                if frame.last() == Some(&b'\r') {
                    frame.pop();
                }
                return Ok(Some(frame));
            }
            if self.buffered.len() > MAX_RPC_FRAME_BYTES {
                return Err(RpcReadError::Oversize);
            }
            if self.eof {
                return if self.buffered.is_empty() {
                    Ok(None)
                } else {
                    Err(RpcReadError::Unterminated)
                };
            }

            let mut chunk = [0_u8; READ_CHUNK_BYTES];
            let read = self.input.read(&mut chunk).await.map_err(map_io_error)?;
            if read == 0 {
                self.eof = true;
            } else {
                self.buffered.extend_from_slice(&chunk[..read]);
            }
        }
    }
}

fn map_io_error(_error: io::Error) -> RpcReadError {
    RpcReadError::Io
}
