use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::geo::CanonicalProvinces;
use crate::util::iso_now;

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_districts.sql")),
];

/// Opens (creating parent directories) with WAL, enforced foreign keys and a
/// busy timeout, so the single-connection service behaves like the previous
/// serialized JSON stores.
pub fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Cannot create database directory {}: {error}",
                    parent.display()
                )
            })?;
        }
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("Cannot open database {}: {error}", path.display()))?;
    // Timeout first, so every later statement waits instead of failing when a
    // second process starts at the same moment.
    connection
        .execute_batch("PRAGMA busy_timeout = 5000;")
        .map_err(|error| format!("Cannot configure database: {error}"))?;
    // Switching to WAL needs a brief exclusive lock; concurrent cold starts can
    // collide here, so retry while the other process finishes its own startup.
    let mut attempts = 0;
    loop {
        match connection.query_row("PRAGMA journal_mode = WAL", [], |row| {
            row.get::<_, String>(0)
        }) {
            Ok(_) => break,
            Err(error) => {
                let busy = matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy)
                        | Some(rusqlite::ErrorCode::DatabaseLocked)
                );
                if busy && attempts < 100 {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    continue;
                }
                return Err(format!("Cannot configure database: {error}"));
            }
        }
    }
    connection
        .execute_batch("PRAGMA foreign_keys = ON;\nPRAGMA synchronous = NORMAL;")
        .map_err(|error| format!("Cannot configure database: {error}"))?;
    Ok(connection)
}

/// Applies every pending versioned migration inside its own transaction and
/// records it in `schema_migrations`.
pub fn migrate(connection: &mut Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (\n\
               version INTEGER PRIMARY KEY,\n\
               applied_at TEXT NOT NULL\n\
             );",
        )
        .map_err(|error| format!("Cannot create migration table: {error}"))?;

    for (version, sql) in MIGRATIONS {
        // The version is re-read under the write lock: two processes
        // cold-starting together must not both apply the same migration.
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("Cannot start migration {version}: {error}"))?;
        let applied: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|error| format!("Cannot read migration state: {error}"))?;
        if *version <= applied {
            drop(transaction);
            continue;
        }
        transaction
            .execute_batch(sql)
            .map_err(|error| format!("Migration {version} failed: {error}"))?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![version, iso_now()],
            )
            .map_err(|error| format!("Cannot record migration {version}: {error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("Cannot commit migration {version}: {error}"))?;
        println!("Applied migration {version}.");
    }
    Ok(())
}

pub fn meta_get(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
}

pub fn meta_set(connection: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO app_meta (key, value) VALUES (?1, ?2)\n\
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn imported_at(connection: &Connection) -> Result<Option<String>, String> {
    meta_get(connection, "bootstrap_imported_at").map_err(|error| error.to_string())
}

/// After the bootstrap import the database must describe exactly the canonical
/// province set; anything else means a partially imported or foreign database.
pub fn verify_provinces(
    connection: &Connection,
    canonical: &CanonicalProvinces,
) -> Result<(), String> {
    let mut statement = connection
        .prepare("SELECT id FROM provinces")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    let mut ids: Vec<String> = Vec::new();
    for row in rows {
        ids.push(row.map_err(|error| error.to_string())?);
    }
    let missing = canonical
        .features
        .iter()
        .any(|feature| !ids.contains(&feature.id));
    if ids.len() != canonical.features.len() || missing {
        return Err(format!(
            "Database province set does not match the canonical GeoJSON ({} database rows, {} canonical features).",
            ids.len(),
            canonical.features.len()
        ));
    }
    Ok(())
}
