//! SQLite records of what was imported: one row per top-level item (copy or link) plus one row
//! per copied file with its size and original modification time.

use crate::db::Db;
use rusqlite::params;
use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ImportRecord {
    pub id: i64,
    pub mode: String,
    pub name: String,
    pub source_path: String,
    pub dest_rel: Option<String>,
    pub is_dir: bool,
    pub files: u64,
    pub bytes: u64,
    pub errors: u64,
    /// running | complete | partial | linked
    pub status: String,
    pub created_at: i64,
}

pub fn insert(
    db: &Db,
    mode: &str,
    name: &str,
    source: &str,
    is_dir: bool,
    status: &str,
) -> Result<i64, String> {
    db.with_conn(|c| {
        c.execute(
            "INSERT INTO imports (mode, name, source_path, is_dir, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![mode, name, source, is_dir, status],
        )?;
        Ok(c.last_insert_rowid())
    })
}

pub fn add_files(db: &Db, id: i64, files: &[(String, u64, Option<u64>)]) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    db.with_conn(|c| {
        let tx = c.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO import_files (import_id, rel_path, size, modified) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (rel, size, modified) in files {
                stmt.execute(params![id, rel, *size as i64, modified.map(|m| m as i64)])?;
            }
        }
        tx.commit()
    })
}

pub fn finish(
    db: &Db,
    id: i64,
    dest_rel: &str,
    files: u64,
    bytes: u64,
    errors: u64,
) -> Result<(), String> {
    let status = if errors == 0 { "complete" } else { "partial" };
    db.with_conn(|c| {
        c.execute(
            "UPDATE imports SET dest_rel = ?2, files = ?3, bytes = ?4, errors = ?5, status = ?6 WHERE id = ?1",
            params![id, dest_rel, files as i64, bytes as i64, errors as i64, status],
        )
        .map(|_| ())
    })
}

/// Removes a record and (by cascade) its file rows.
pub fn delete(db: &Db, id: i64) -> Result<(), String> {
    db.with_conn(|c| {
        c.execute("DELETE FROM imports WHERE id = ?1", params![id])
            .map(|_| ())
    })
}

pub fn list(db: &Db) -> Result<Vec<ImportRecord>, String> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, mode, name, source_path, dest_rel, is_dir, files, bytes, errors, status, created_at
             FROM imports ORDER BY id DESC LIMIT 200",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ImportRecord {
                id: r.get(0)?,
                mode: r.get(1)?,
                name: r.get(2)?,
                source_path: r.get(3)?,
                dest_rel: r.get(4)?,
                is_dir: r.get(5)?,
                files: r.get::<_, i64>(6)? as u64,
                bytes: r.get::<_, i64>(7)? as u64,
                errors: r.get::<_, i64>(8)? as u64,
                status: r.get(9)?,
                created_at: r.get(10)?,
            })
        })?;
        rows.collect()
    })
}

#[cfg(test)]
pub fn file_count(db: &Db, id: i64) -> Result<u64, String> {
    db.with_conn(|c| {
        c.query_row(
            "SELECT COUNT(*) FROM import_files WHERE import_id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        )
    })
    .map(|n| n as u64)
}

/// Rows still marked `running` belong to a job the app died during; their staged data is gone.
pub fn drop_interrupted(db: &Db) -> Result<(), String> {
    db.with_conn(|c| {
        c.execute("DELETE FROM imports WHERE status = 'running'", [])
            .map(|_| ())
    })
}

/// `(id, source path)` of every active link, to restore at startup.
pub fn links(db: &Db) -> Result<Vec<(i64, String)>, String> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, source_path FROM imports WHERE mode = 'link' AND status = 'linked'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_roundtrip_and_cascade() {
        let db = Db::open_in_memory().unwrap();
        let id = insert(&db, "copy", "proj", "C:\\src\\proj", true, "running").unwrap();
        add_files(
            &db,
            id,
            &[
                ("a.txt".into(), 5, Some(100)),
                ("sub/b.bin".into(), 9, None),
            ],
        )
        .unwrap();
        assert_eq!(file_count(&db, id).unwrap(), 2);
        finish(&db, id, "proj", 2, 14, 0).unwrap();
        let l = list(&db).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(
            (l[0].status.as_str(), l[0].files, l[0].bytes),
            ("complete", 2, 14)
        );
        assert_eq!(l[0].dest_rel.as_deref(), Some("proj"));
        delete(&db, id).unwrap();
        assert_eq!(
            file_count(&db, id).unwrap(),
            0,
            "file rows go with their import"
        );
    }

    #[test]
    fn errors_make_an_import_partial_and_interrupted_rows_are_dropped() {
        let db = Db::open_in_memory().unwrap();
        let a = insert(&db, "copy", "a", "s", false, "running").unwrap();
        let b = insert(&db, "copy", "b", "s", false, "running").unwrap();
        finish(&db, a, "a", 1, 1, 3).unwrap();
        drop_interrupted(&db).unwrap();
        let l = list(&db).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].id, l[0].status.as_str()), (a, "partial"));
        assert_ne!(l[0].id, b);
    }

    #[test]
    fn only_active_links_are_restored() {
        let db = Db::open_in_memory().unwrap();
        insert(&db, "link", "x", "C:\\x", true, "linked").unwrap();
        insert(&db, "copy", "y", "C:\\y", true, "complete").unwrap();
        let ls = links(&db).unwrap();
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].1, "C:\\x");
    }
}
