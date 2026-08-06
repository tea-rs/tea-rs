use std::collections::BTreeMap;

use crate::{CodingError, CodingErrorCode};

pub(crate) const MAX_RESOURCE_BYTES: usize = 128 * 1024;
const MAX_FRONTMATTER_BYTES: usize = 16 * 1024;

pub(crate) struct FrontmatterDocument {
    pub(crate) fields: BTreeMap<String, String>,
    pub(crate) body: String,
}

pub(crate) fn parse(source: &str) -> Result<FrontmatterDocument, CodingError> {
    if source.len() > MAX_RESOURCE_BYTES || !source.starts_with("---\n") {
        return Err(invalid());
    }
    let remaining = &source[4..];
    let end = remaining.find("\n---\n").ok_or_else(invalid)?;
    if end > MAX_FRONTMATTER_BYTES {
        return Err(invalid());
    }
    let mut fields = BTreeMap::new();
    for line in remaining[..end].lines() {
        let (key, value) = line.split_once(':').ok_or_else(invalid)?;
        let key = key.trim();
        let value = value.trim();
        if !valid_key(key)
            || value.is_empty()
            || value.len() > 4096
            || value.chars().any(char::is_control)
            || fields.insert(key.to_owned(), value.to_owned()).is_some()
        {
            return Err(invalid());
        }
    }
    let body = remaining[end + 5..].to_owned();
    if body.is_empty() || body.contains('\0') {
        return Err(invalid());
    }
    Ok(FrontmatterDocument { fields, body })
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub(crate) fn field<'a>(
    document: &'a FrontmatterDocument,
    name: &str,
) -> Result<&'a str, CodingError> {
    document
        .fields
        .get(name)
        .map(String::as_str)
        .ok_or_else(invalid)
}

pub(crate) fn invalid() -> CodingError {
    CodingError::new(
        CodingErrorCode::InvalidInput,
        "declarative resource is invalid",
    )
}
