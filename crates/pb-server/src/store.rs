//! Boards, their pictures and access tokens in one SQLite file.

use std::path::Path;

use pb_core::AssetId;
use rusqlite::{Connection, OptionalExtension, params};

/// What a token lets its holder do with a board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    View,
    Edit,
}

pub struct Store {
    db: Connection,
}

/// A new board's id and its two links' tokens.
#[derive(Clone, Debug)]
pub struct NewBoard {
    pub board: String,
    pub edit: String,
    pub view: String,
}

/// Random text for ids and tokens: `bytes` random bytes in hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the system has randomness");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Tokens are kept hashed: the file alone does not give access.
fn token_hash(token: &str) -> Vec<u8> {
    blake3::hash(token.as_bytes()).as_bytes().to_vec()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The database's layout, one step per version: step `i` takes a database from version `i`
/// (in `PRAGMA user_version`) to `i + 1`. Never change a step that was released; add one.
const MIGRATIONS: &[&str] = &[
    // 1: boards, their pictures and tokens. Databases from before versions had these
    // already, at version 0: the step leaves them as they are.
    "CREATE TABLE IF NOT EXISTS boards (
         id TEXT PRIMARY KEY,
         created INTEGER NOT NULL,
         updated INTEGER NOT NULL,
         snapshot BLOB
     );
     CREATE TABLE IF NOT EXISTS assets (
         board TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
         id BLOB NOT NULL,
         bytes BLOB NOT NULL,
         PRIMARY KEY (board, id)
     );
     CREATE TABLE IF NOT EXISTS tokens (
         hash BLOB PRIMARY KEY,
         board TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
         can_edit INTEGER NOT NULL,
         created INTEGER NOT NULL
     );",
];

/// Brings `db` to the latest layout, each step whole or not at all. A database from a
/// newer server is refused rather than misread.
fn migrate(db: &mut Connection) -> rusqlite::Result<()> {
    migrate_with(db, MIGRATIONS)
}

/// The layout version `db` is at.
fn layout(db: &Connection) -> rusqlite::Result<usize> {
    let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(usize::try_from(version).unwrap_or(usize::MAX))
}

fn migrate_with(db: &mut Connection, steps: &[&str]) -> rusqlite::Result<()> {
    let version = layout(db)?;
    if version > steps.len() {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "the database is from a newer server (layout {version}, this one knows {})",
            steps.len()
        )));
    }
    for (i, step) in steps.iter().enumerate().skip(version) {
        let tx = db.transaction()?;
        tx.execute_batch(step)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
        log::info!("database layout {} ready", i + 1);
    }
    Ok(())
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut db: Connection) -> rusqlite::Result<Self> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;",
        )?;
        migrate(&mut db)?;
        Ok(Self { db })
    }

    /// Makes an empty board with a link to edit it and one to look at it.
    pub fn create_board(&self) -> rusqlite::Result<NewBoard> {
        let new = NewBoard {
            board: random_hex(12),
            edit: random_hex(16),
            view: random_hex(16),
        };
        let now = now();
        self.db.execute(
            "INSERT INTO boards (id, created, updated) VALUES (?1, ?2, ?2)",
            params![new.board, now],
        )?;
        for (token, edit) in [(&new.edit, true), (&new.view, false)] {
            self.db.execute(
                "INSERT INTO tokens (hash, board, can_edit, created) VALUES (?1, ?2, ?3, ?4)",
                params![token_hash(token), new.board, edit, now],
            )?;
        }
        Ok(new)
    }

    pub fn board_count(&self) -> rusqlite::Result<usize> {
        self.db
            .query_row("SELECT COUNT(*) FROM boards", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|n| n as usize)
    }

    /// Writes a consistent copy of the whole database to `path` (replaced if there).
    pub fn back_up(&self, path: &Path) -> rusqlite::Result<()> {
        let _ = std::fs::remove_file(path);
        self.db
            .execute("VACUUM INTO ?1", params![path.to_string_lossy()])
            .map(drop)
    }

    /// What `token` allows on `board`; `None` if nothing.
    pub fn access(&self, board: &str, token: &str) -> rusqlite::Result<Option<Access>> {
        let edit: Option<bool> = self
            .db
            .query_row(
                "SELECT can_edit FROM tokens WHERE hash = ?1 AND board = ?2",
                params![token_hash(token), board],
                |row| row.get(0),
            )
            .optional()?;
        Ok(edit.map(|e| if e { Access::Edit } else { Access::View }))
    }

    /// The board's stored state, if the board exists: its snapshot (none before the
    /// first edit) and its pictures.
    #[allow(clippy::type_complexity)]
    pub fn load(&self, board: &str) -> rusqlite::Result<Option<(Option<Vec<u8>>, Vec<Vec<u8>>)>> {
        let snapshot: Option<Option<Vec<u8>>> = self
            .db
            .query_row(
                "SELECT snapshot FROM boards WHERE id = ?1",
                params![board],
                |row| row.get(0),
            )
            .optional()?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let mut query = self
            .db
            .prepare("SELECT bytes FROM assets WHERE board = ?1")?;
        let assets = query
            .query_map(params![board], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<Vec<u8>>>>()?;
        Ok(Some((snapshot, assets)))
    }

    /// Stores the board and pictures it does not have yet, in one transaction.
    pub fn save(
        &mut self,
        board: &str,
        snapshot: &[u8],
        assets: &[(AssetId, &[u8])],
    ) -> rusqlite::Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE boards SET snapshot = ?2, updated = ?3 WHERE id = ?1",
            params![board, snapshot, now()],
        )?;
        for (id, bytes) in assets {
            tx.execute(
                "INSERT OR IGNORE INTO assets (board, id, bytes) VALUES (?1, ?2, ?3)",
                params![board, id.as_bytes().as_slice(), bytes],
            )?;
        }
        tx.commit()
    }

    /// The pictures of `board` already stored.
    #[cfg(test)]
    pub fn asset_ids(&self, board: &str) -> rusqlite::Result<Vec<AssetId>> {
        let mut query = self.db.prepare("SELECT id FROM assets WHERE board = ?1")?;
        let ids = query
            .query_map(params![board], |row| row.get::<_, Vec<u8>>(0))?
            .filter_map(|id| {
                let bytes: [u8; 32] = id.ok()?.try_into().ok()?;
                Some(AssetId::from_bytes(bytes))
            })
            .collect();
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_step_up_and_newer_ones_are_refused() {
        let mut db = Connection::open_in_memory().unwrap();
        // A database from before versions: tables there, version 0, a board in it.
        db.execute_batch(MIGRATIONS[0]).unwrap();
        db.execute(
            "INSERT INTO boards (id, created, updated) VALUES ('old', 1, 1)",
            [],
        )
        .unwrap();
        let next = "ALTER TABLE boards ADD COLUMN name TEXT;";
        migrate_with(&mut db, &[MIGRATIONS[0], next]).unwrap();
        let version = layout(&db).unwrap();
        assert_eq!(version, 2);
        let kept: String = db
            .query_row("SELECT id FROM boards WHERE name IS NULL", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(kept, "old");
        // Run again, nothing happens; a server knowing fewer steps refuses it.
        migrate_with(&mut db, &[MIGRATIONS[0], next]).unwrap();
        assert!(migrate_with(&mut db, MIGRATIONS).is_err());
        // A failing step leaves the database as it was.
        let mut db = Connection::open_in_memory().unwrap();
        assert!(migrate_with(&mut db, &[MIGRATIONS[0], "CREATE TABLE x (a); nonsense"]).is_err());
        let version = layout(&db).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn tokens_give_their_access_to_their_board_only() {
        let store = Store::in_memory().unwrap();
        let a = store.create_board().unwrap();
        let b = store.create_board().unwrap();
        assert_eq!(store.access(&a.board, &a.edit).unwrap(), Some(Access::Edit));
        assert_eq!(store.access(&a.board, &a.view).unwrap(), Some(Access::View));
        assert_eq!(store.access(&b.board, &a.edit).unwrap(), None);
        assert_eq!(store.access(&a.board, "guess").unwrap(), None);
    }

    #[test]
    fn boards_are_saved_and_loaded() {
        let mut store = Store::in_memory().unwrap();
        let new = store.create_board().unwrap();
        assert_eq!(store.load(&new.board).unwrap(), Some((None, vec![])));
        let picture = b"png".as_slice();
        store
            .save(&new.board, b"snapshot", &[(AssetId::of(picture), picture)])
            .unwrap();
        store
            .save(
                &new.board,
                b"snapshot 2",
                &[(AssetId::of(picture), picture)],
            )
            .unwrap();
        let (snapshot, assets) = store.load(&new.board).unwrap().unwrap();
        assert_eq!(snapshot.as_deref(), Some(b"snapshot 2".as_slice()));
        assert_eq!(assets, vec![picture.to_vec()]);
        assert_eq!(
            store.asset_ids(&new.board).unwrap(),
            vec![AssetId::of(picture)]
        );
        assert_eq!(store.load("nope").unwrap(), None);
    }
}
