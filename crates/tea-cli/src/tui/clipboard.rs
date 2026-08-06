use std::io::{self, Write as _};
use std::process::{Command, Stdio};

const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Explicit outward clipboard boundary used only by interactive commands.
pub trait Clipboard {
    /// Copies one bounded UTF-8 value.
    ///
    /// # Errors
    ///
    /// Returns an I/O failure when no clipboard integration is available.
    fn copy(&mut self, text: &str) -> io::Result<()>;
}

/// In-memory clipboard for hermetic tests and embedding.
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard {
    contents: Option<String>,
}

impl MemoryClipboard {
    /// Returns the last copied value.
    #[must_use]
    pub fn contents(&self) -> Option<&str> {
        self.contents.as_deref()
    }
}

impl Clipboard for MemoryClipboard {
    fn copy(&mut self, text: &str) -> io::Result<()> {
        validate(text)?;
        self.contents = Some(text.to_owned());
        Ok(())
    }
}

/// Host clipboard adapter using the platform's standard non-shell command.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&mut self, text: &str) -> io::Result<()> {
        validate(text)?;
        #[cfg(target_os = "macos")]
        {
            pipe_to("pbcopy", &[], text)
        }
        #[cfg(target_os = "windows")]
        {
            pipe_to("clip", &[], text)
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            pipe_to("wl-copy", &[], text)
                .or_else(|_| pipe_to("xclip", &["-selection", "clipboard"], text))
        }
        #[cfg(not(any(unix, target_os = "windows")))]
        {
            let _ = text;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "system clipboard is unavailable",
            ))
        }
    }
}

fn validate(text: &str) -> io::Result<()> {
    if text.len() > MAX_CLIPBOARD_BYTES || text.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "clipboard text is invalid",
        ));
    }
    Ok(())
}

fn pipe_to(program: &str, arguments: &[&str], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clipboard stdin is unavailable"))?
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("clipboard command failed"))
    }
}
