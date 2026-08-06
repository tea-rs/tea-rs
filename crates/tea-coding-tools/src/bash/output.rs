use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{FileToolError, FileToolErrorCode};

pub(crate) const MAX_CAPTURE_BYTES_PER_STREAM: usize = 16 * 1024;
pub(crate) const MAX_SPILL_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_PROGRESS_EVENTS: usize = 64;
static NEXT_OUTPUT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub(crate) enum OutputKind {
    Stdout,
    Stderr,
}

impl OutputKind {
    const fn label(self) -> &'static [u8] {
        match self {
            Self::Stdout => b"[stdout] ",
            Self::Stderr => b"[stderr] ",
        }
    }
}

pub(crate) struct OutputCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    spill: Option<SpillFile>,
    spill_bytes: usize,
    truncated: bool,
}

impl OutputCapture {
    pub(crate) const fn new() -> Self {
        Self {
            stdout: Vec::new(),
            stderr: Vec::new(),
            spill: None,
            spill_bytes: 0,
            truncated: false,
        }
    }

    pub(crate) fn push(
        &mut self,
        kind: OutputKind,
        chunk: &[u8],
        directory: &Path,
    ) -> Result<(), FileToolError> {
        let memory = match kind {
            OutputKind::Stdout => &mut self.stdout,
            OutputKind::Stderr => &mut self.stderr,
        };
        let available = MAX_CAPTURE_BYTES_PER_STREAM.saturating_sub(memory.len());
        let retained = available.min(chunk.len());
        memory.extend_from_slice(&chunk[..retained]);
        let overflow = &chunk[retained..];
        if overflow.is_empty() {
            return Ok(());
        }
        self.truncated = true;
        let additional = kind.label().len().saturating_add(overflow.len());
        if self.spill_bytes.saturating_add(additional) > MAX_SPILL_BYTES {
            return Err(FileToolError::new(FileToolErrorCode::OutputLimit));
        }
        let spill = match &mut self.spill {
            Some(spill) => spill,
            None => self.spill.insert(SpillFile::create(directory)?),
        };
        spill.write(kind.label())?;
        spill.write(overflow)?;
        self.spill_bytes += additional;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<CapturedOutput, FileToolError> {
        let overflow_reference = self.spill.as_mut().map(SpillFile::persist).transpose()?;
        Ok(CapturedOutput {
            stdout: canonical_text(&self.stdout),
            stderr: canonical_text(&self.stderr),
            overflow_reference,
            truncated: self.truncated,
        })
    }
}

fn canonical_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\0', "�")
}

pub(crate) struct CapturedOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) overflow_reference: Option<String>,
    pub(crate) truncated: bool,
}

struct SpillFile {
    file: File,
    path: std::path::PathBuf,
    reference: String,
    persisted: bool,
}

impl SpillFile {
    fn create(directory: &Path) -> Result<Self, FileToolError> {
        for _ in 0..32 {
            let id = NEXT_OUTPUT_ID.fetch_add(1, Ordering::Relaxed);
            let reference = format!("bash-output-{}-{id}.log", std::process::id());
            let path = directory.join(&reference);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        path,
                        reference,
                        persisted: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(FileToolError::new(FileToolErrorCode::FilesystemFailure));
                }
            }
        }
        Err(FileToolError::new(FileToolErrorCode::FilesystemFailure))
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), FileToolError> {
        self.file
            .write_all(bytes)
            .map_err(|_| FileToolError::new(FileToolErrorCode::FilesystemFailure))
    }

    fn persist(&mut self) -> Result<String, FileToolError> {
        self.file
            .sync_all()
            .map_err(|_| FileToolError::new(FileToolErrorCode::FilesystemFailure))?;
        self.persisted = true;
        Ok(self.reference.clone())
    }
}

impl Drop for SpillFile {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
