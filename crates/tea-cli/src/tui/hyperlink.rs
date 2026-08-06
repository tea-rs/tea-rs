use std::io;
use std::io::Write;
use std::sync::Arc;

use ratatui::layout::{Position, Rect};

pub(crate) const MAX_OSC8_DESTINATION_BYTES: usize = 2_048;
pub(crate) const OSC8_CLOSE: &[u8] = b"\x1b]8;;\x1b\\";

pub(crate) type Hyperlink = Arc<str>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HyperlinkBuffer {
    area: Rect,
    content: Vec<Option<Hyperlink>>,
}

impl HyperlinkBuffer {
    pub(crate) fn empty(area: Rect) -> Self {
        Self {
            area,
            content: vec![None; area_len(area)],
        }
    }

    pub(crate) fn resize(&mut self, area: Rect) {
        self.area = area;
        self.content.clear();
        self.content.resize(area_len(area), None);
    }

    pub(crate) fn reset(&mut self) {
        self.content.fill(None);
    }

    pub(crate) fn set(&mut self, position: Position, link: Option<Hyperlink>) {
        let Some(index) = self.index_of(position) else {
            return;
        };
        self.content[index] = link;
    }

    pub(crate) fn set_range(&mut self, position: Position, width: usize, link: Option<&Hyperlink>) {
        for offset in 0..width {
            let Ok(offset) = u16::try_from(offset) else {
                break;
            };
            self.set(
                Position::new(position.x.saturating_add(offset), position.y),
                link.cloned(),
            );
        }
    }

    pub(crate) fn get(&self, position: Position) -> Option<&Hyperlink> {
        self.index_of(position)
            .and_then(|index| self.content[index].as_ref())
    }

    fn index_of(&self, position: Position) -> Option<usize> {
        if !self.area.contains(position) {
            return None;
        }
        let row = usize::from(position.y.saturating_sub(self.area.y));
        let column = usize::from(position.x.saturating_sub(self.area.x));
        Some(row * usize::from(self.area.width) + column)
    }
}

pub(crate) fn write_open(writer: &mut impl Write, destination: &str) -> io::Result<bool> {
    if validate_destination(destination).as_deref() != Some(destination) {
        return Ok(false);
    }
    writer.write_all(b"\x1b]8;;")?;
    writer.write_all(destination.as_bytes())?;
    writer.write_all(b"\x1b\\")?;
    Ok(true)
}

pub(crate) fn write_close(writer: &mut impl Write) -> io::Result<()> {
    writer.write_all(OSC8_CLOSE)
}

pub(crate) fn validate_destination(destination: &str) -> Option<String> {
    if destination.is_empty()
        || destination.len() > MAX_OSC8_DESTINATION_BYTES
        || destination.trim() != destination
        || destination.chars().any(char::is_control)
    {
        return None;
    }
    let parsed = url::Url::parse(destination).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.cannot_be_a_base()
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    let normalized = parsed.to_string();
    (normalized.len() <= MAX_OSC8_DESTINATION_BYTES).then_some(normalized)
}

fn area_len(area: Rect) -> usize {
    usize::from(area.width) * usize::from(area.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_rejects_unstructured_or_protocol_unsafe_destinations() {
        for destination in [
            "",
            "file:///tmp/demo",
            "https://user:secret@example.test/",
            "HTTPS://EXAMPLE.TEST",
            "https://example.test/\u{1b}]8;;https://evil.test",
        ] {
            let mut output = Vec::new();
            assert!(!write_open(&mut output, destination).unwrap());
            assert!(output.is_empty());
        }
    }
}
