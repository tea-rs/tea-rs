use std::{fmt, io::Write as _, time::Duration};

use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    time::Instant,
};

const READ_CHUNK_BYTES: usize = 8 * 1024;
const UTF8_BOM: &[u8; 3] = b"\xEF\xBB\xBF";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameError {
    Io,
    TooLarge,
    Incomplete,
    Malformed,
    Deadline,
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io => "frame I/O failed",
            Self::TooLarge => "frame exceeded its byte bound",
            Self::Incomplete => "frame ended before its delimiter",
            Self::Malformed => "frame was malformed",
            Self::Deadline => "frame I/O exceeded its deadline",
        })
    }
}

impl std::error::Error for FrameError {}

pub(crate) struct BoundedFrameReader<R> {
    reader: R,
    frame: Vec<u8>,
    chunk: [u8; READ_CHUNK_BYTES],
    chunk_start: usize,
    chunk_end: usize,
    frame_started_at: Option<Instant>,
    max_frame_bytes: usize,
    frame_timeout: Duration,
}

impl<R> BoundedFrameReader<R>
where
    R: AsyncRead + Unpin,
{
    pub(crate) fn new(reader: R, max_frame_bytes: usize, frame_timeout: Duration) -> Self {
        Self {
            reader,
            frame: Vec::with_capacity(max_frame_bytes.min(READ_CHUNK_BYTES)),
            chunk: [0; READ_CHUNK_BYTES],
            chunk_start: 0,
            chunk_end: 0,
            frame_started_at: None,
            max_frame_bytes,
            frame_timeout,
        }
    }

    pub(crate) async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, FrameError> {
        loop {
            if self.chunk_start == self.chunk_end {
                let read = if let Some(started_at) = self.frame_started_at {
                    tokio::time::timeout_at(
                        started_at + self.frame_timeout,
                        self.reader.read(&mut self.chunk),
                    )
                    .await
                    .map_err(|_| FrameError::Deadline)?
                } else {
                    self.reader.read(&mut self.chunk).await
                }
                .map_err(|_| FrameError::Io)?;
                if read == 0 {
                    return if self.frame.is_empty() {
                        Ok(None)
                    } else {
                        Err(FrameError::Incomplete)
                    };
                }
                self.chunk_start = 0;
                self.chunk_end = read;
            }

            let byte = self.chunk[self.chunk_start];
            self.chunk_start += 1;
            if byte == b'\n' {
                if self.frame.last() == Some(&b'\r') {
                    self.frame.pop();
                }
                self.frame_started_at = None;
                if self.frame.is_empty() {
                    continue;
                }
                let parsed = {
                    let frame = self
                        .frame
                        .strip_prefix(UTF8_BOM.as_slice())
                        .unwrap_or(&self.frame);
                    serde_json::from_slice(frame).map_err(|_| FrameError::Malformed)
                };
                self.frame.clear();
                return parsed.map(Some);
            }

            if self.frame.is_empty() {
                self.frame_started_at = Some(Instant::now());
            }
            if self.frame.len() == self.max_frame_bytes {
                return Err(FrameError::TooLarge);
            }
            self.frame.push(byte);
        }
    }

    #[cfg(test)]
    fn retained_bytes(&self) -> usize {
        self.frame.len()
    }
}

pub(crate) fn serialize<T: Serialize>(
    value: &T,
    max_frame_bytes: usize,
) -> Result<Vec<u8>, FrameError> {
    let mut writer = BoundedWriter::new(max_frame_bytes);
    let result = serde_json::to_writer(&mut writer, value);
    if writer.overflowed {
        return Err(FrameError::TooLarge);
    }
    result.map_err(|_| FrameError::Malformed)?;
    writer.bytes.write_all(b"\n").map_err(|_| FrameError::Io)?;
    Ok(writer.bytes)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    overflowed: bool,
}

impl BoundedWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_bytes.min(READ_CHUNK_BYTES)),
            max_bytes,
            overflowed: false,
        }
    }
}

impl std::io::Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if buffer.len() > remaining {
            self.bytes.extend_from_slice(&buffer[..remaining]);
            self.overflowed = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame bound exceeded",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::{Value, json};
    use tokio::io::{AsyncWriteExt, duplex};

    use super::{BoundedFrameReader, FrameError, serialize};

    #[tokio::test]
    async fn reader_never_retains_more_than_the_frame_bound() {
        let (mut sender, receiver) = duplex(16 * 1024);
        sender.write_all(&vec![b'x'; 4_097]).await.unwrap();
        let mut reader = BoundedFrameReader::new(receiver, 4_096, Duration::from_secs(1));

        assert_eq!(reader.read::<Value>().await, Err(FrameError::TooLarge));
        assert_eq!(reader.retained_bytes(), 4_096);
    }

    #[tokio::test]
    async fn incomplete_and_malformed_frames_are_distinct() {
        let (mut sender, receiver) = duplex(64);
        sender.write_all(b"{\"open\":true").await.unwrap();
        drop(sender);
        let mut reader = BoundedFrameReader::new(receiver, 64, Duration::from_secs(1));
        assert_eq!(reader.read::<Value>().await, Err(FrameError::Incomplete));

        let (mut sender, receiver) = duplex(64);
        sender.write_all(b"not-json\n").await.unwrap();
        let mut reader = BoundedFrameReader::new(receiver, 64, Duration::from_secs(1));
        assert_eq!(reader.read::<Value>().await, Err(FrameError::Malformed));
    }

    #[tokio::test]
    async fn partial_frame_has_one_absolute_deadline() {
        let (mut sender, receiver) = duplex(64);
        sender.write_all(b"{").await.unwrap();
        let mut reader = BoundedFrameReader::new(receiver, 64, Duration::from_millis(25));
        assert_eq!(reader.read::<Value>().await, Err(FrameError::Deadline));
    }

    #[test]
    fn serialization_fails_without_an_unbounded_intermediate_buffer() {
        let value = json!({"value": "x".repeat(4_096)});
        assert_eq!(serialize(&value, 512), Err(FrameError::TooLarge));
        let frame = serialize(&json!({"ok": true}), 512).unwrap();
        assert_eq!(frame.last(), Some(&b'\n'));
        assert!(frame.len() <= 513);
    }
}
