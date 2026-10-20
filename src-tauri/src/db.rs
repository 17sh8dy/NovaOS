use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

const MAX_KEY_LEN: usize = 128;
const MAX_VALUE_BYTES: usize = 1024 * 1024;

/// SQLite-backed key/value store for settings, profiles and small app data.
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS kv (
                 key   TEXT PRIMARY KEY NOT NULL,
                 value TEXT NOT NULL,
                 updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
             );
             CREATE TABLE IF NOT EXISTS imports (
                 id          INTEGER PRIMARY KEY AUTOINCREMENT,
                 mode        TEXT NOT NULL,
                 name        TEXT NOT NULL,
                 source_path TEXT NOT NULL,
                 dest_rel    TEXT,
                 is_dir      INTEGER NOT NULL,
                 files       INTEGER NOT NULL DEFAULT 0,
                 bytes       INTEGER NOT NULL DEFAULT 0,
                 errors      INTEGER NOT NULL DEFAULT 0,
                 status      TEXT NOT NULL,
                 created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now'))
             );
             CREATE TABLE IF NOT EXISTS import_files (
                 import_id INTEGER NOT NULL REFERENCES imports(id) ON DELETE CASCADE,
                 rel_path  TEXT NOT NULL,
                 size      INTEGER NOT NULL,
                 modified  INTEGER,
                 PRIMARY KEY (import_id, rel_path)
             ) WITHOUT ROWID;",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run a closure with the connection. Used by the import store for multi-statement work.
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut conn = self.conn.lock().map_err(|_| "database lock poisoned")?;
        f(&mut conn).map_err(|e| e.to_string())
    }

    pub fn get(&self, key: &str) -> Result<Option<String>, String> {
        validate_key(key)?;
        let conn = self.conn.lock().map_err(|_| "database lock poisoned")?;
        conn.query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<(), String> {
        validate_key(key)?;
        if value.len() > MAX_VALUE_BYTES {
            return Err(format!("value too large (max {MAX_VALUE_BYTES} bytes)"));
        }
        let conn = self.conn.lock().map_err(|_| "database lock poisoned")?;
        conn.execute(
            "INSERT INTO kv (key, value, updated_at) VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

/// Keys are namespaced identifiers like `theme` or `notes.main`; nothing exotic gets in.
fn validate_key(key: &str) -> Result<(), String> {
    let ok = !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err("invalid key: use 1-128 chars of letters, digits, '.', '_' or '-'".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_overwrite() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.get("theme").unwrap(), None);
        db.set("theme", "a").unwrap();
        db.set("theme", "b").unwrap();
        assert_eq!(db.get("theme").unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn rejects_bad_keys() {
        let db = Db::open_in_memory().unwrap();
        for k in [
            "",
            "has space",
            "a/b",
            "x'; DROP TABLE kv;--",
            &"k".repeat(129),
        ] {
            assert!(db.set(k, "v").is_err(), "{k:?} should be rejected");
            assert!(db.get(k).is_err());
        }
        assert!(db.set("notes.main", "ok").is_ok());
    }

    #[test]
    fn rejects_oversized_values() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.set("big", &"x".repeat(MAX_VALUE_BYTES + 1)).is_err());
        assert!(db.set("big", &"x".repeat(MAX_VALUE_BYTES)).is_ok());
    }

    #[test]
    fn values_with_quotes_are_stored_verbatim() {
        let db = Db::open_in_memory().unwrap();
        let v = "it's \"quoted\"; DROP TABLE kv;";
        db.set("q", v).unwrap();
        assert_eq!(db.get("q").unwrap().as_deref(), Some(v));
    }
}
