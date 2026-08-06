use crossterm::event::{Event, EventStream, KeyEvent, KeyEventKind};
use futures_util::StreamExt as _;
use tokio::sync::mpsc;

/// Bounded terminal input projected into the interactive application loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// Key press or repeat; release events are discarded by the input pump.
    Key(KeyEvent),
    /// One bracketed paste payload.
    Paste(String),
    /// New terminal dimensions.
    Resize {
        /// Display-cell columns.
        width: u16,
        /// Terminal rows.
        height: u16,
    },
    /// Terminal focus changed.
    Focus(bool),
}

/// Owned cancellable crossterm input task.
#[derive(Debug)]
pub struct InputPump {
    handle: tokio::task::JoinHandle<()>,
}

impl InputPump {
    /// Cancels and awaits the owned input reader.
    pub async fn shutdown(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

/// Starts one bounded crossterm input reader.
///
/// # Panics
///
/// Panics when `capacity` is zero.
#[must_use]
pub fn spawn_input_pump(capacity: usize) -> (InputPump, mpsc::Receiver<InputEvent>) {
    assert!(capacity > 0, "input capacity must be non-zero");
    let (sender, receiver) = mpsc::channel(capacity);
    let handle = tokio::spawn(async move {
        let mut events = EventStream::new();
        while let Some(event) = events.next().await {
            let Ok(event) = event else {
                break;
            };
            let projected = match event {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    Some(InputEvent::Key(key))
                }
                Event::Paste(text) => Some(InputEvent::Paste(text)),
                Event::Resize(width, height) => Some(InputEvent::Resize { width, height }),
                Event::FocusGained => Some(InputEvent::Focus(true)),
                Event::FocusLost => Some(InputEvent::Focus(false)),
                Event::Key(_) | Event::Mouse(_) => None,
            };
            if let Some(event) = projected
                && sender.send(event).await.is_err()
            {
                break;
            }
        }
    });
    (InputPump { handle }, receiver)
}
