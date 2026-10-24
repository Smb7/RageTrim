use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::error::Error;
use crate::models::{Clip, Folder};

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn list_folders(&self) -> Result<Vec<Folder>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM folders ORDER BY path COLLATE NOCASE")?;
        let rows = stmt.query_map([], |row| {
            Ok(Folder {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn add_folder(&self, path: &str) -> Result<Folder, Error> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO folders (path, created_at) VALUES (?1, ?2)
             ON CONFLICT(path) DO NOTHING",
            params![path, now],
        )?;
        self.folder_by_path(path)?
            .ok_or_else(|| Error::message("Couldn't save that folder."))
    }

    pub fn remove_folder(&self, id: i64) -> Result<(), Error> {
        self.conn
            .execute("DELETE FROM folders WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn folder_by_path(&self, path: &str) -> Result<Option<Folder>, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path FROM folders WHERE path = ?1")?;
        let mut rows = stmt.query(params![path])?;
        match rows.next()? {
            Some(row) => Ok(Some(Folder {
                id: row.get(0)?,
                path: row.get(1)?,
            })),
            None => Ok(None),
        }
    }

    pub fn upsert_clip(
        &self,
        folder_id: i64,
        path: &str,
        file_name: &str,
        size_bytes: i64,
        modified_ms: i64,
    ) -> Result<Upsert, Error> {
        let existing = self.conn.query_row(
            "SELECT size_bytes, modified_ms, missing FROM clips WHERE path = ?1",
            params![path],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        );
        match existing {
            Ok((size, modified, missing)) => {
                let changed = size != size_bytes || modified != modified_ms || missing != 0;
                if changed {
                    self.conn.execute(
                        "UPDATE clips
                         SET folder_id = ?1, file_name = ?2, size_bytes = ?3,
                             modified_ms = ?4, missing = 0
                         WHERE path = ?5",
                        params![folder_id, file_name, size_bytes, modified_ms, path],
                    )?;
                    Ok(Upsert::Updated)
                } else {
                    Ok(Upsert::Unchanged)
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                self.conn.execute(
                    "INSERT INTO clips (folder_id, path, file_name, size_bytes, modified_ms, missing)
                     VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                    params![folder_id, path, file_name, size_bytes, modified_ms],
                )?;
                Ok(Upsert::Added)
            }
            Err(err) => Err(err.into()),
        }
    }

    pub fn mark_missing_unseen(
        &self,
        folder_id: i64,
        seen: &HashSet<String>,
    ) -> Result<u32, Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM clips WHERE folder_id = ?1 AND missing = 0")?;
        let present = stmt.query_map(params![folder_id], |row| row.get::<_, String>(0))?;
        let mut missing = Vec::new();
        for path in present {
            let path = path?;
            if !seen.contains(&path) {
                missing.push(path);
            }
        }
        drop(stmt);

        if missing.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut update = tx.prepare("UPDATE clips SET missing = 1 WHERE path = ?1")?;
            for path in &missing {
                update.execute(params![path])?;
            }
        }
        tx.commit()?;
        Ok(missing.len() as u32)
    }

    pub fn list_clips(&self) -> Result<Vec<Clip>, Error> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.folder_id, f.path, c.path, c.file_name, c.size_bytes,
                    c.modified_ms, c.viewed_at, c.missing, c.duration_ms, c.width,
                    c.height, c.video_codec, c.edited_at
             FROM clips c
             JOIN folders f ON f.id = c.folder_id
             ORDER BY c.missing ASC, c.modified_ms DESC, c.file_name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], map_clip)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Error::from)
    }

    pub fn set_viewed(&self, id: i64, viewed: bool) -> Result<Clip, Error> {
        let viewed_at = viewed.then(now_ms);
        let changed = self.conn.execute(
            "UPDATE clips SET viewed_at = ?1 WHERE id = ?2",
            params![viewed_at, id],
        )?;
        if changed == 0 {
            return Err(Error::message("That clip is no longer in the library."));
        }
        self.clip_by_id(id)
    }

    pub fn set_edited(&self, id: i64, edited: bool) -> Result<Clip, Error> {
        let edited_at = edited.then(now_ms);
        let changed = self.conn.execute(
            "UPDATE clips SET edited_at = ?1 WHERE id = ?2",
            params![edited_at, id],
        )?;
        if changed == 0 {
            return Err(Error::message("That clip is no longer in the library."));
        }
        self.clip_by_id(id)
    }

    pub fn save_probe(
        &self,
        id: i64,
        duration_ms: Option<i64>,
        width: Option<i64>,
        height: Option<i64>,
        video_codec: Option<&str>,
    ) -> Result<Clip, Error> {
        let changed = self.conn.execute(
            "UPDATE clips
             SET duration_ms = ?1, width = ?2, height = ?3, video_codec = ?4
             WHERE id = ?5",
            params![duration_ms, width, height, video_codec, id],
        )?;
        if changed == 0 {
            return Err(Error::message("That clip is no longer in the library."));
        }
        self.clip_by_id(id)
    }

    pub fn record_export(
        &self,
        clip_id: i64,
        path: &str,
        in_ms: i64,
        out_ms: i64,
        mode: &str,
    ) -> Result<Clip, Error> {
        let now = now_ms();
        self.conn.execute(
            "INSERT INTO exports (clip_id, path, in_ms, out_ms, mode, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![clip_id, path, in_ms, out_ms, mode, now],
        )?;
        self.conn.execute(
            "UPDATE clips SET edited_at = ?1 WHERE id = ?2",
            params![now, clip_id],
        )?;
        self.clip_by_id(clip_id)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, Error> {
        match self.conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        ) {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), Error> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn export_dir(&self) -> Result<PathBuf, Error> {
        if let Some(path) = self.get_setting("export_dir")? {
            return Ok(PathBuf::from(path));
        }
        default_export_dir()
    }

    pub fn compress_target_bytes(&self) -> Result<u64, Error> {
        let Some(value) = self.get_setting("compress_target_bytes")? else {
            return Ok(crate::ffmpeg::DEFAULT_TARGET_BYTES);
        };
        value
            .parse()
            .map_err(|_| Error::message("The compress target is invalid."))
    }

    pub fn clip_by_id(&self, id: i64) -> Result<Clip, Error> {
        self.conn
            .query_row(
                "SELECT c.id, c.folder_id, f.path, c.path, c.file_name, c.size_bytes,
                        c.modified_ms, c.viewed_at, c.missing, c.duration_ms, c.width,
                        c.height, c.video_codec, c.edited_at
                 FROM clips c
                 JOIN folders f ON f.id = c.folder_id
                 WHERE c.id = ?1",
                params![id],
                map_clip,
            )
            .map_err(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => {
                    Error::message("That clip is no longer in the library.")
                }
                other => other.into(),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upsert {
    Added,
    Updated,
    Unchanged,
}

fn map_clip(row: &rusqlite::Row<'_>) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        folder_path: row.get(2)?,
        path: row.get(3)?,
        file_name: row.get(4)?,
        size_bytes: row.get(5)?,
        modified_ms: row.get(6)?,
        viewed_at: row.get(7)?,
        missing: row.get::<_, i64>(8)? != 0,
        duration_ms: row.get(9)?,
        width: row.get(10)?,
        height: row.get(11)?,
        video_codec: row.get(12)?,
        edited_at: row.get(13)?,
    })
}

fn migrate(conn: &Connection) -> Result<(), Error> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "
            CREATE TABLE folders (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE clips (
                id INTEGER PRIMARY KEY,
                folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                path TEXT NOT NULL UNIQUE,
                file_name TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                modified_ms INTEGER NOT NULL,
                viewed_at INTEGER,
                missing INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_clips_folder ON clips(folder_id);
            CREATE INDEX idx_clips_modified ON clips(modified_ms);
            ",
        )?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        conn.execute_batch(
            "
            ALTER TABLE clips ADD COLUMN duration_ms INTEGER;
            ALTER TABLE clips ADD COLUMN width INTEGER;
            ALTER TABLE clips ADD COLUMN height INTEGER;
            ALTER TABLE clips ADD COLUMN video_codec TEXT;
            ALTER TABLE clips ADD COLUMN edited_at INTEGER;
            CREATE TABLE settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE exports (
                id INTEGER PRIMARY KEY,
                clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                in_ms INTEGER NOT NULL,
                out_ms INTEGER NOT NULL,
                mode TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            ",
        )?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    Ok(())
}

pub fn default_export_dir() -> Result<PathBuf, Error> {
    if let Some(videos) = dirs::video_dir() {
        return Ok(videos.join("RageTrim"));
    }
    Ok(app_data_dir()?.join("exports"))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub fn app_data_dir() -> Result<PathBuf, Error> {
    let base =
        dirs::data_dir().ok_or_else(|| Error::message("Couldn't find the app data folder."))?;
    Ok(base.join("RageTrim"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (Database, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("ragetrim-db-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::open(&dir.join("test.db")).unwrap();
        (db, dir)
    }

    #[test]
    fn viewed_mark_round_trips_and_folder_delete_drops_clips() {
        let (db, dir) = temp_db();
        let folder = db.add_folder(r"C:\Clips").unwrap();
        assert_eq!(db.add_folder(r"C:\Clips").unwrap().id, folder.id);

        let upsert = db
            .upsert_clip(folder.id, r"C:\Clips\loss.mp4", "loss.mp4", 12, 99)
            .unwrap();
        assert_eq!(upsert, Upsert::Added);
        let again = db
            .upsert_clip(folder.id, r"C:\Clips\loss.mp4", "loss.mp4", 12, 99)
            .unwrap();
        assert_eq!(again, Upsert::Unchanged);

        let clip = db.list_clips().unwrap().pop().unwrap();
        assert!(clip.viewed_at.is_none());
        let viewed = db.set_viewed(clip.id, true).unwrap();
        assert!(viewed.viewed_at.is_some());
        let cleared = db.set_viewed(clip.id, false).unwrap();
        assert!(cleared.viewed_at.is_none());

        db.remove_folder(folder.id).unwrap();
        assert!(db.list_clips().unwrap().is_empty());
        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}
