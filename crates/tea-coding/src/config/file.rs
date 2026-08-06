use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};
use tea_protocol::{ModelRef, ReasoningEffort};

use crate::config::SettingsLayer;
use crate::{CodingError, CodingErrorCode};

/// Maximum encoded settings file bytes.
pub const MAX_SETTINGS_FILE_BYTES: usize = 256 * 1024;

static SETTINGS_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Loads one optional strict JSON settings layer.
///
/// # Errors
///
/// Rejects oversized, invalid UTF-8/JSON, unknown-field, and unsupported-version files.
pub fn load_settings_file(path: &Path) -> Result<Option<SettingsLayer>, CodingError> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    parse_settings(&bytes).map(Some)
}

/// Atomically updates only the global model and reasoning defaults.
///
/// Existing unrelated sparse settings are preserved. A missing file is created
/// with the current schema version and private permissions.
///
/// # Errors
///
/// Rejects invalid or oversized existing settings and reports durable-write
/// failures without replacing the old file before the atomic rename succeeds.
pub fn persist_global_model_settings(
    path: &Path,
    model: &ModelRef,
    reasoning_effort: ReasoningEffort,
) -> Result<(), CodingError> {
    let mut document = match read_optional(path)? {
        Some(bytes) => {
            parse_settings(&bytes)?;
            serde_json::from_slice::<Value>(&bytes).map_err(|_| invalid())?
        }
        None => Value::Object(Map::from_iter([(
            "schemaVersion".to_owned(),
            Value::from(crate::config::CODING_SETTINGS_SCHEMA_VERSION),
        )])),
    };
    let object = document.as_object_mut().ok_or_else(invalid)?;
    object.insert(
        "provider".to_owned(),
        Value::String(model.provider_id().to_string()),
    );
    object.insert(
        "model".to_owned(),
        Value::String(model.model_id().to_string()),
    );
    object.insert(
        "thinking".to_owned(),
        Value::String(reasoning_effort.as_str().to_owned()),
    );
    let mut encoded = serde_json::to_vec_pretty(&document).map_err(|_| persistence_write())?;
    encoded.push(b'\n');
    if encoded.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(invalid());
    }
    atomic_write(path, &encoded).map_err(|_| persistence_write())
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, CodingError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(persistence()),
    };
    if bytes.len() > MAX_SETTINGS_FILE_BYTES {
        return Err(invalid());
    }
    Ok(Some(bytes))
}

fn parse_settings(bytes: &[u8]) -> Result<SettingsLayer, CodingError> {
    let layer = serde_json::from_slice::<SettingsLayer>(bytes).map_err(|_| invalid())?;
    if layer
        .schema_version
        .is_some_and(|version| version != crate::config::CODING_SETTINGS_SCHEMA_VERSION)
    {
        return Err(invalid());
    }
    Ok(layer)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(path, bytes, |from, to| fs::rename(from, to))
}

fn atomic_write_with(
    path: &Path,
    bytes: &[u8],
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing settings filename"))?;
    let (temporary_path, mut temporary) = create_temporary(parent, file_name)?;
    let result = (|| {
        temporary.write_all(bytes)?;
        temporary.sync_all()?;
        drop(temporary);
        rename(&temporary_path, path)?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_temporary(parent: &Path, file_name: &std::ffi::OsStr) -> io::Result<(PathBuf, File)> {
    for _ in 0..32 {
        let sequence = SETTINGS_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
        let temporary_path = parent.join(temporary_name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&temporary_path) {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "settings temporary file is unavailable",
    ))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn invalid() -> CodingError {
    CodingError::new(CodingErrorCode::InvalidInput, "settings file is invalid")
}

fn persistence() -> CodingError {
    CodingError::new(CodingErrorCode::Persistence, "settings file cannot be read")
}

fn persistence_write() -> CodingError {
    CodingError::new(
        CodingErrorCode::Persistence,
        "settings file cannot be persisted",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_rename_leaves_existing_settings_intact() {
        let root = std::env::temp_dir().join(format!(
            "tea-coding-settings-rename-{}-{}",
            std::process::id(),
            SETTINGS_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let original = br#"{"schemaVersion":1,"model":"old"}"#;
        fs::write(&path, original).unwrap();

        let result = atomic_write_with(&path, b"replacement", |_, _| {
            Err(io::Error::other("injected rename failure"))
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
