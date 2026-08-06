use std::fmt;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tea_protocol::{ContentBlock, MAX_INLINE_IMAGE_BASE64_BYTES};
use thiserror::Error;

/// Maximum image attachments retained in one session composer.
pub const MAX_COMPOSER_ATTACHMENTS: usize = 4;
/// Maximum aggregate encoded image bytes retained in one session composer.
pub const MAX_COMPOSER_IMAGE_BASE64_BYTES: usize = MAX_INLINE_IMAGE_BASE64_BYTES;
pub(crate) const MAX_INLINE_IMAGE_RAW_BYTES: usize = MAX_INLINE_IMAGE_BASE64_BYTES / 4 * 3;
pub(crate) const MAX_ATTACHMENT_DISPLAY_NAME_BYTES: usize = 96;

/// Safe failure returned while validating a local composer attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AttachmentError {
    /// The local file could not be opened or read.
    #[error("image could not be read")]
    Read,
    /// One image exceeds the per-block protocol limit.
    #[error("image exceeds the attachment size limit")]
    TooLarge,
    /// The detected or declared MIME type is not supported by the TUI.
    #[error("image format is unsupported")]
    UnsupportedFormat,
    /// The composer already contains the maximum number of attachments.
    #[error("composer attachment limit reached")]
    TooMany,
    /// Adding the image would exceed the aggregate composer byte limit.
    #[error("composer attachment bytes exceed the limit")]
    TotalTooLarge,
    /// The encoded payload is empty, malformed, or rejected by the protocol.
    #[error("image content is invalid")]
    InvalidContent,
}

/// One validated, path-free image retained in local composer state.
#[derive(Clone, PartialEq)]
pub struct ComposerAttachment {
    content: ContentBlock,
    mime_type: &'static str,
    display_name: String,
    decoded_bytes: usize,
    encoded_bytes: usize,
}

impl ComposerAttachment {
    /// Builds an attachment from a standard padded Base64 image payload.
    ///
    /// # Errors
    ///
    /// Returns [`AttachmentError`] when the MIME type is unsupported or the
    /// payload violates the protocol image contract.
    pub fn inline(
        mime_type: &str,
        data: impl Into<String>,
        display_name: impl AsRef<Path>,
    ) -> Result<Self, AttachmentError> {
        let mime_type = supported_mime(mime_type).ok_or(AttachmentError::UnsupportedFormat)?;
        let data = data.into();
        if data.is_empty() {
            return Err(AttachmentError::InvalidContent);
        }
        if data.len() > MAX_INLINE_IMAGE_BASE64_BYTES {
            return Err(AttachmentError::TooLarge);
        }
        let decoded_bytes = STANDARD
            .decode(data.as_bytes())
            .map_err(|_| AttachmentError::InvalidContent)?
            .len();
        let encoded_bytes = data.len();
        let content = ContentBlock::inline_image(mime_type, data)
            .map_err(|_| AttachmentError::InvalidContent)?;

        Ok(Self {
            content,
            mime_type,
            display_name: safe_display_name(display_name.as_ref()),
            decoded_bytes,
            encoded_bytes,
        })
    }

    /// Returns the canonical protocol content block.
    #[must_use]
    pub const fn content(&self) -> &ContentBlock {
        &self.content
    }

    /// Returns the signature-derived MIME type.
    #[must_use]
    pub const fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    /// Returns the bounded path-free display name.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Returns the decoded payload size in bytes.
    #[must_use]
    pub const fn decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }

    /// Returns the encoded payload size in bytes.
    #[must_use]
    pub const fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
}

impl fmt::Debug for ComposerAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerAttachment")
            .field("mime_type", &self.mime_type)
            .field("display_name", &self.display_name)
            .field("decoded_bytes", &self.decoded_bytes)
            .field("encoded_bytes", &self.encoded_bytes)
            .finish_non_exhaustive()
    }
}

pub(crate) fn load_local_image(path: &Path) -> Result<ComposerAttachment, AttachmentError> {
    let file = File::open(path).map_err(|_| AttachmentError::Read)?;
    let mut bytes = Vec::new();
    file.take((MAX_INLINE_IMAGE_RAW_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AttachmentError::Read)?;
    let decoded_bytes = bytes.len();
    let encoded_bytes = encoded_len(bytes.len())?;
    let mime_type = detect_mime(&bytes).ok_or(AttachmentError::UnsupportedFormat)?;
    let encoded = STANDARD.encode(bytes);
    debug_assert_eq!(encoded.len(), encoded_bytes);
    let attachment = ComposerAttachment::inline(mime_type, encoded, path)?;
    debug_assert_eq!(attachment.decoded_bytes(), decoded_bytes);
    debug_assert_eq!(attachment.encoded_bytes(), encoded_bytes);
    Ok(attachment)
}

pub(crate) fn validate_addition(
    current_count: usize,
    current_encoded_bytes: usize,
    added_encoded_bytes: usize,
) -> Result<(), AttachmentError> {
    if current_count >= MAX_COMPOSER_ATTACHMENTS {
        return Err(AttachmentError::TooMany);
    }
    if current_encoded_bytes
        .checked_add(added_encoded_bytes)
        .is_none_or(|total| total > MAX_COMPOSER_IMAGE_BASE64_BYTES)
    {
        return Err(AttachmentError::TotalTooLarge);
    }
    Ok(())
}

pub(crate) fn decoded_inline_image_bytes(data: &str) -> Option<usize> {
    if data.is_empty() || !data.len().is_multiple_of(4) {
        return None;
    }
    let padding = if data.ends_with("==") {
        2
    } else {
        usize::from(data.ends_with('='))
    };
    data.len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

pub(crate) fn format_byte_count(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format_decimal_bytes(bytes, KIB, "KiB")
    } else {
        format_decimal_bytes(bytes, MIB, "MiB")
    }
}

pub(crate) fn format_compact_byte_count(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;
    if bytes < KIB {
        format!("{bytes}B")
    } else if bytes < MIB {
        format!("{}K", bytes.div_ceil(KIB))
    } else {
        format!("{}M", bytes.div_ceil(MIB))
    }
}

fn format_decimal_bytes(bytes: usize, unit: usize, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = bytes % unit * 10 / unit;
    if decimal == 0 {
        format!("{whole} {suffix}")
    } else {
        format!("{whole}.{decimal} {suffix}")
    }
}

fn encoded_len(raw_bytes: usize) -> Result<usize, AttachmentError> {
    if raw_bytes > MAX_INLINE_IMAGE_RAW_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    raw_bytes
        .checked_add(2)
        .map(|rounded| rounded / 3 * 4)
        .ok_or(AttachmentError::TooLarge)
}

fn detect_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn supported_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("image/png"),
        "image/jpeg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

fn safe_display_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map_or_else(|| "image".into(), |name| name.to_string_lossy());
    let mut result = String::new();
    let mut pending_space = false;
    for character in raw.chars().filter(|character| !character.is_control()) {
        if character.is_whitespace() {
            pending_space = !result.is_empty();
            continue;
        }
        let needed = character.len_utf8() + usize::from(pending_space);
        if result.len() + needed > MAX_ATTACHMENT_DISPLAY_NAME_BYTES {
            break;
        }
        if pending_space {
            result.push(' ');
            pending_space = false;
        }
        result.push(character);
    }
    if result.is_empty() {
        "image".to_owned()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use tea_protocol::{ContentBlock, ImageSource, MAX_INLINE_IMAGE_BASE64_BYTES};

    use super::*;

    static ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tea-tui-attachment-{}-{}-{name}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn loader_recognizes_supported_signatures_without_trusting_extension() {
        let fixtures: [(&str, &[u8]); 4] = [
            ("image/png", b"\x89PNG\r\n\x1a\nbody"),
            ("image/jpeg", b"\xff\xd8\xffbody"),
            ("image/gif", b"GIF89abody"),
            ("image/webp", b"RIFF\x04\x00\x00\x00WEBPbody"),
        ];

        for (mime_type, bytes) in fixtures {
            let path = fixture_path("misleading.txt");
            fs::write(&path, bytes).unwrap();
            let attachment = load_local_image(&path).unwrap();

            assert_eq!(attachment.mime_type(), mime_type);
            assert_eq!(attachment.decoded_bytes(), bytes.len());
            assert!(attachment.display_name().ends_with("misleading.txt"));
            assert!(matches!(
                attachment.content(),
                ContentBlock::Image {
                    mime_type: content_mime,
                    source: ImageSource::InlineBase64 { .. },
                } if content_mime == mime_type
            ));
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn loader_rejects_empty_unsupported_and_oversized_content() {
        for bytes in [b"".as_slice(), b"not an image".as_slice()] {
            let path = fixture_path("invalid.bin");
            fs::write(&path, bytes).unwrap();
            assert_eq!(
                load_local_image(&path).unwrap_err(),
                AttachmentError::UnsupportedFormat
            );
            fs::remove_file(path).unwrap();
        }

        assert_eq!(
            encoded_len(MAX_INLINE_IMAGE_RAW_BYTES).unwrap(),
            MAX_INLINE_IMAGE_BASE64_BYTES
        );
        assert_eq!(
            encoded_len(MAX_INLINE_IMAGE_RAW_BYTES + 1),
            Err(AttachmentError::TooLarge)
        );
    }

    #[test]
    fn display_name_is_bounded_and_control_safe() {
        let long = format!("{}\nsecret.png", "界".repeat(80));
        let name = safe_display_name(Path::new(&long));

        assert!(name.len() <= MAX_ATTACHMENT_DISPLAY_NAME_BYTES);
        assert!(!name.chars().any(char::is_control));
        assert!(name.is_char_boundary(name.len()));
        assert_eq!(safe_display_name(Path::new("\n\t")), "image");
    }

    #[test]
    fn attachment_limits_bound_count_and_aggregate_bytes() {
        assert_eq!(validate_addition(0, 0, 4), Ok(()));
        assert_eq!(
            validate_addition(MAX_COMPOSER_ATTACHMENTS, 0, 4),
            Err(AttachmentError::TooMany)
        );
        assert_eq!(
            validate_addition(1, MAX_COMPOSER_IMAGE_BASE64_BYTES - 3, 4),
            Err(AttachmentError::TotalTooLarge)
        );
    }

    #[test]
    fn attachment_debug_omits_encoded_content() {
        let path = fixture_path("debug.png");
        fs::write(&path, b"\x89PNG\r\n\x1a\nprivate-payload").unwrap();
        let attachment = load_local_image(&path).unwrap();
        let debug = format!("{attachment:?}");

        assert!(debug.contains("image/png"));
        assert!(!debug.contains("private-payload"));
        assert!(!debug.contains("iVBOR"));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn inline_sizes_are_derived_without_decoding_the_payload() {
        assert_eq!(decoded_inline_image_bytes("iVBORw0KGgo="), Some(8));
        assert_eq!(decoded_inline_image_bytes("/9j/"), Some(3));
        assert_eq!(decoded_inline_image_bytes("bad"), None);
        assert_eq!(format_byte_count(8), "8 B");
        assert_eq!(format_byte_count(1536), "1.5 KiB");
        assert_eq!(format_compact_byte_count(1536), "2K");
    }
}
