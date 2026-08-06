//! Initial `SQLite` schema installation and validation.

use rusqlite::{Connection, OptionalExtension as _, TransactionBehavior};

/// The schema version written by this crate.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

const SCHEMA_VERSION_TABLE: TableSpec = TableSpec {
    name: "schema_version",
    columns: &[ColumnSpec::primary_key("version", "INTEGER", 1)],
};

const REQUIRED_TABLES: &[TableSpec] = &[
    SCHEMA_VERSION_TABLE,
    TableSpec {
        name: "records",
        columns: &[
            ColumnSpec::not_null_primary_key("session_id", "TEXT", 1),
            ColumnSpec::not_null_primary_key("sequence", "INTEGER", 2),
            ColumnSpec::not_null("record_id", "TEXT"),
            ColumnSpec::not_null("envelope", "TEXT"),
        ],
    },
    TableSpec {
        name: "approval_artifacts",
        columns: &[
            ColumnSpec::not_null_primary_key("session_id", "TEXT", 1),
            ColumnSpec::not_null_primary_key("record_id", "TEXT", 2),
            ColumnSpec::not_null("envelope", "TEXT"),
        ],
    },
    TableSpec {
        name: "grant_journal",
        columns: &[
            ColumnSpec::not_null_primary_key("session_id", "TEXT", 1),
            ColumnSpec::not_null_primary_key("seq", "INTEGER", 2),
            ColumnSpec::not_null("grant_id", "TEXT"),
            ColumnSpec::not_null("envelope", "TEXT"),
        ],
    },
    TableSpec {
        name: "active_grants",
        columns: &[
            ColumnSpec::primary_key("grant_id", "TEXT", 1),
            ColumnSpec::not_null("session_id", "TEXT"),
            ColumnSpec::not_null("actor_id", "TEXT"),
            ColumnSpec::not_null("grant_json", "TEXT"),
            ColumnSpec::not_null("revoked", "INTEGER"),
        ],
    },
    TableSpec {
        name: "session_catalog",
        columns: &[
            ColumnSpec::primary_key("session_id", "TEXT", 1),
            ColumnSpec::nullable("display_name", "TEXT"),
        ],
    },
];

const REQUIRED_INDEXES: &[IndexSpec] = &[
    IndexSpec {
        name: "idx_records_record_id",
        table: "records",
        unique: true,
        columns: &["session_id", "record_id"],
    },
    IndexSpec {
        name: "idx_grant_journal_grant_id",
        table: "grant_journal",
        unique: false,
        columns: &["grant_id"],
    },
    IndexSpec {
        name: "idx_active_grants_actor_revoked",
        table: "active_grants",
        unique: false,
        columns: &["actor_id", "revoked", "grant_id"],
    },
];

/// Installs the complete initial schema or verifies an existing published layout.
///
/// Pre-0.1 development schemas are intentionally not migrated. Since some of
/// them also used version 1, both the version row and the complete layout must
/// match before the database is accepted.
pub(crate) fn ensure_schema(conn: &mut Connection) -> Result<(), rusqlite::Error> {
    let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if !schema_object_exists(&transaction, "table", "schema_version", "schema_version")? {
        if has_user_schema_objects(&transaction)? {
            return Err(schema_error(
                "incompatible SQLite schema: schema_version is missing from a non-empty database",
            ));
        }
        create_schema(&transaction)?;
    }
    validate_schema(&transaction)?;
    transaction.commit()
}

fn create_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE schema_version (
            version INTEGER PRIMARY KEY
        );
        CREATE TABLE records (
            session_id TEXT NOT NULL,
            sequence INTEGER NOT NULL,
            record_id TEXT NOT NULL,
            envelope TEXT NOT NULL,
            PRIMARY KEY (session_id, sequence)
        );
        CREATE UNIQUE INDEX idx_records_record_id
            ON records (session_id, record_id);
        CREATE TABLE approval_artifacts (
            session_id TEXT NOT NULL,
            record_id TEXT NOT NULL,
            envelope TEXT NOT NULL,
            PRIMARY KEY (session_id, record_id)
        );
        CREATE TABLE grant_journal (
            session_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            grant_id TEXT NOT NULL,
            envelope TEXT NOT NULL,
            PRIMARY KEY (session_id, seq)
        );
        CREATE INDEX idx_grant_journal_grant_id
            ON grant_journal (grant_id);
        CREATE TABLE active_grants (
            grant_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            actor_id TEXT NOT NULL,
            grant_json TEXT NOT NULL,
            revoked INTEGER NOT NULL CHECK (revoked IN (0, 1))
        );
        CREATE INDEX idx_active_grants_actor_revoked
            ON active_grants (actor_id, revoked, grant_id);
        CREATE TABLE session_catalog (
            session_id TEXT PRIMARY KEY,
            display_name TEXT
        );",
    )?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?)",
        [CURRENT_SCHEMA_VERSION],
    )?;
    Ok(())
}

fn validate_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    validate_table(conn, &SCHEMA_VERSION_TABLE)?;
    let installed = read_single_schema_version(conn)?;
    if installed != i64::from(CURRENT_SCHEMA_VERSION) {
        return Err(schema_error(format!(
            "unsupported SQLite schema version {installed}; expected {CURRENT_SCHEMA_VERSION}"
        )));
    }
    for table in &REQUIRED_TABLES[1..] {
        validate_table(conn, table)?;
    }
    for index in REQUIRED_INDEXES {
        validate_index(conn, index)?;
    }
    Ok(())
}

fn validate_table(conn: &Connection, expected: &TableSpec) -> Result<(), rusqlite::Error> {
    if !schema_object_exists(conn, "table", expected.name, expected.name)? {
        return Err(schema_error(format!(
            "incompatible SQLite schema version 1: required table `{}` is missing",
            expected.name
        )));
    }
    let mut statement = conn.prepare(
        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden
         FROM pragma_table_xinfo(?1, 'main') ORDER BY cid",
    )?;
    let columns = statement
        .query_map([expected.name], |row| {
            Ok(ColumnMetadata {
                position: row.get(0)?,
                name: row.get(1)?,
                data_type: row.get(2)?,
                not_null: row.get(3)?,
                default_value: row.get(4)?,
                primary_key_position: row.get(5)?,
                hidden: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.len() != expected.columns.len()
        || columns
            .iter()
            .zip(expected.columns)
            .enumerate()
            .any(|(position, (actual, expected))| !actual.matches(position, expected))
    {
        return Err(malformed_table(expected.name));
    }
    Ok(())
}

fn validate_index(conn: &Connection, expected: &IndexSpec) -> Result<(), rusqlite::Error> {
    if !schema_object_exists(conn, "index", expected.name, expected.table)? {
        return Err(schema_error(format!(
            "incompatible SQLite schema version 1: required index `{}` is missing",
            expected.name
        )));
    }
    let metadata = conn
        .query_row(
            "SELECT \"unique\", origin, partial
             FROM pragma_index_list(?1, 'main') WHERE name = ?2",
            [expected.table, expected.name],
            |row| {
                Ok(IndexMetadata {
                    unique: row.get(0)?,
                    origin: row.get(1)?,
                    partial: row.get(2)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| malformed_index(expected.name))?;
    if metadata.unique != expected.unique || metadata.origin != "c" || metadata.partial {
        return Err(malformed_index(expected.name));
    }

    let mut statement = conn.prepare(
        "SELECT name, desc, coll
         FROM pragma_index_xinfo(?1, 'main') WHERE \"key\" = 1 ORDER BY seqno",
    )?;
    let columns = statement
        .query_map([expected.name], |row| {
            Ok(IndexColumnMetadata {
                name: row.get(0)?,
                descending: row.get(1)?,
                collation: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if columns.len() != expected.columns.len()
        || columns
            .iter()
            .zip(expected.columns)
            .any(|(actual, expected)| {
                actual.name != *expected || actual.descending || actual.collation != "BINARY"
            })
    {
        return Err(malformed_index(expected.name));
    }
    Ok(())
}

fn read_single_schema_version(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let mut statement =
        conn.prepare("SELECT version, typeof(version) FROM schema_version ORDER BY rowid")?;
    let mut rows = statement.query([])?;
    let Some(row) = rows.next()? else {
        return Err(malformed_version_row());
    };
    let value_type: String = row.get(1)?;
    if value_type != "integer" {
        return Err(malformed_version_row());
    }
    let version = row.get(0)?;
    if rows.next()?.is_some() {
        return Err(malformed_version_row());
    }
    Ok(version)
}

fn schema_object_exists(
    conn: &Connection,
    object_type: &str,
    name: &str,
    table: &str,
) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM main.sqlite_schema
            WHERE type = ?1 AND name = ?2 AND tbl_name = ?3
        )",
        [object_type, name, table],
        |row| row.get(0),
    )
}

fn has_user_schema_objects(conn: &Connection) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM main.sqlite_schema WHERE name NOT GLOB 'sqlite_*'
        )",
        [],
        |row| row.get(0),
    )
}

fn malformed_table(name: &str) -> rusqlite::Error {
    schema_error(format!(
        "incompatible SQLite schema version 1: table `{name}` has an unexpected layout"
    ))
}

fn malformed_index(name: &str) -> rusqlite::Error {
    schema_error(format!(
        "incompatible SQLite schema version 1: index `{name}` has an unexpected layout"
    ))
}

fn malformed_version_row() -> rusqlite::Error {
    schema_error("incompatible SQLite schema: schema_version must contain exactly one integer row")
}

fn schema_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_SCHEMA),
        Some(message.into()),
    )
}

#[derive(Clone, Copy)]
struct TableSpec {
    name: &'static str,
    columns: &'static [ColumnSpec],
}

#[derive(Clone, Copy)]
struct ColumnSpec {
    name: &'static str,
    data_type: &'static str,
    not_null: bool,
    primary_key_position: u32,
}

impl ColumnSpec {
    const fn nullable(name: &'static str, data_type: &'static str) -> Self {
        Self {
            name,
            data_type,
            not_null: false,
            primary_key_position: 0,
        }
    }

    const fn not_null(name: &'static str, data_type: &'static str) -> Self {
        Self {
            name,
            data_type,
            not_null: true,
            primary_key_position: 0,
        }
    }

    const fn primary_key(
        name: &'static str,
        data_type: &'static str,
        primary_key_position: u32,
    ) -> Self {
        Self {
            name,
            data_type,
            not_null: false,
            primary_key_position,
        }
    }

    const fn not_null_primary_key(
        name: &'static str,
        data_type: &'static str,
        primary_key_position: u32,
    ) -> Self {
        Self {
            name,
            data_type,
            not_null: true,
            primary_key_position,
        }
    }
}

struct ColumnMetadata {
    position: u32,
    name: String,
    data_type: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key_position: u32,
    hidden: u32,
}

impl ColumnMetadata {
    fn matches(&self, position: usize, expected: &ColumnSpec) -> bool {
        usize::try_from(self.position).ok() == Some(position)
            && self.name == expected.name
            && self.data_type == expected.data_type
            && self.not_null == expected.not_null
            && self.default_value.is_none()
            && self.primary_key_position == expected.primary_key_position
            && self.hidden == 0
    }
}

struct IndexSpec {
    name: &'static str,
    table: &'static str,
    unique: bool,
    columns: &'static [&'static str],
}

struct IndexMetadata {
    unique: bool,
    origin: String,
    partial: bool,
}

struct IndexColumnMetadata {
    name: String,
    descending: bool,
    collation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incompatible_layout_uses_sqlite_schema_error_code() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY);
                 INSERT INTO schema_version (version) VALUES (1);",
            )
            .unwrap();

        let error = ensure_schema(&mut connection).unwrap_err();
        match error {
            rusqlite::Error::SqliteFailure(error, Some(message)) => {
                assert_eq!(error.extended_code, rusqlite::ffi::SQLITE_SCHEMA);
                assert!(message.contains("required table `records` is missing"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }
}
