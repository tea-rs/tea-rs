//! Minimal `.env` file loader for tests (no `dotenv` dependency, no `unsafe`).
//!
//! Parses `KEY=VALUE` lines, ignoring blank lines and `#` comments, stripping
//! surrounding double quotes from values. The loader returns a `BTreeMap`; the
//! live smoke test builds a `MapCredentialResolver` from it rather than mutating
//! the process environment (which would require `unsafe` `set_var` under the
//! `unsafe_code = "forbid"` workspace lint).

use std::collections::BTreeMap;
use std::path::Path;

/// Parses a `.env` file into a `KEY -> VALUE` map.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub fn load_env_file(path: &Path) -> Result<BTreeMap<String, String>, std::io::Error> {
    let contents = std::fs::read_to_string(path)?;
    let mut map = BTreeMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_owned();
        if key.is_empty() {
            continue;
        }
        let mut value = value.trim().to_owned();
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            value.remove(0);
            value.pop();
        }
        map.insert(key, value);
    }
    Ok(map)
}
