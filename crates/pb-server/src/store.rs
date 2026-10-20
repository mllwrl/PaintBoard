//! Boards, their pictures and access tokens in one SQLite file.

use std::path::Path;

use pb_core::{AssetId, Role};
use rusqlite::{Connection, OptionalExtension, params};

/// How a role is stored.
fn role_number(role: Role) -> i64 {
    match role {
        Role::View => 0,
        Role::Comment => 1,
        Role::Edit => 2,
    }
}

fn role_of(number: i64) -> Role {
    match number {
        2.. => Role::Edit,
        1 => Role::Comment,
        _ => Role::View,
    }
}

pub struct Store {
    db: Connection,
}

/// A new board's id and its links' tokens.
#[derive(Clone, Debug)]
pub struct NewBoard {
    pub board: String,
    pub edit: String,
    pub comment: String,
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
    // 2: links to comment: what a token allows is its role (0 look, 1 comment, 2 edit);
    // `can_edit` stays, set for editing links.
    "ALTER TABLE tokens ADD COLUMN role INTEGER NOT NULL DEFAULT 0;
     UPDATE tokens SET role = 2 WHERE can_edit = 1;",
    // 3: accounts. People are their email addresses. A board's members are kept by
    // address, so someone can be let in before they first sign in; role 3 is the owner.
    // Sign-ins wait in `logins` until the link in the email is followed. Boards keep their
    // name for lists of them.
    "CREATE TABLE users (
         email TEXT PRIMARY KEY,
         created INTEGER NOT NULL
     );
     CREATE TABLE sessions (
         hash BLOB PRIMARY KEY,
         email TEXT NOT NULL REFERENCES users(email) ON DELETE CASCADE,
         created INTEGER NOT NULL,
         used INTEGER NOT NULL
     );
     CREATE TABLE logins (
         request BLOB PRIMARY KEY,
         link BLOB NOT NULL UNIQUE,
         email TEXT NOT NULL,
         code TEXT NOT NULL,
         created INTEGER NOT NULL,
         confirmed INTEGER NOT NULL DEFAULT 0
     );
     CREATE TABLE members (
         board TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
         email TEXT NOT NULL,
         role INTEGER NOT NULL,
         PRIMARY KEY (board, email)
     );
     CREATE INDEX members_by_email ON members(email);
     ALTER TABLE boards ADD COLUMN title TEXT;",
];

/// How long the link in a sign-in email works, in seconds.
pub const LOGIN_TTL: i64 = 15 * 60;

/// A session unused this long (seconds) is signed out.
pub const SESSION_TTL: i64 = 180 * 86_400;

/// Stored role number of a board's owner.
const OWNER: i64 = 3;

/// What someone is on a board through their account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Membership {
    pub role: Role,
    /// Made the board: may let people in, make new links and delete it.
    pub owner: bool,
}

impl Membership {
    fn of(number: i64) -> Self {
        Self {
            role: role_of(number),
            owner: number >= OWNER,
        }
    }

    /// As in answers: `owner`, `edit`, `comment` or `view`.
    pub fn name(self) -> &'static str {
        if self.owner {
            return "owner";
        }
        role_name(self.role)
    }
}

pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::View => "view",
        Role::Comment => "comment",
        Role::Edit => "edit",
    }
}

pub fn role_named(name: &str) -> Option<Role> {
    match name {
        "view" => Some(Role::View),
        "comment" => Some(Role::Comment),
        "edit" => Some(Role::Edit),
        _ => None,
    }
}

/// A sign-in just started: the secret the app asks with, the one in the email's link,
/// and a short code shown in both, so the person can tell the email is for their app.
#[derive(Clone, Debug)]
pub struct Login {
    pub request: String,
    pub link: String,
    pub code: String,
}

/// Where a sign-in is, asked by the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginState {
    /// The link in the email was not followed yet.
    Waiting,
    /// Signed in: the new session and whose it is.
    Done { session: String, email: String },
    /// Expired, used already, or never there.
    Gone,
}

/// A board in someone's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardEntry {
    pub board: String,
    pub title: Option<String>,
    pub membership: Membership,
    pub updated: i64,
}

/// `text` as an email address kept by the server (trimmed, lower case), if it looks like
/// one.
pub fn email_address(text: &str) -> Option<String> {
    let email = text.trim().to_lowercase();
    let (name, host) = email.split_once('@')?;
    let fine = !name.is_empty()
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && email.len() <= 254
        && !email[name.len() + 1..].contains('@')
        && !email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "<>\",;:".contains(c));
    fine.then_some(email)
}

/// A short code to compare by eye: no letters that look alike.
fn login_code() -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 4];
    getrandom::fill(&mut buf).expect("the system has randomness");
    buf.iter()
        .map(|b| char::from(LETTERS[*b as usize % LETTERS.len()]))
        .collect()
}

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

    /// Makes an empty board with links to edit it, to comment on it and to look at it;
    /// `owner`, if signed in, owns it.
    pub fn create_board(&self, owner: Option<&str>) -> rusqlite::Result<NewBoard> {
        let new = NewBoard {
            board: random_hex(12),
            edit: random_hex(16),
            comment: random_hex(16),
            view: random_hex(16),
        };
        let now = now();
        self.db.execute(
            "INSERT INTO boards (id, created, updated) VALUES (?1, ?2, ?2)",
            params![new.board, now],
        )?;
        for (token, role) in [
            (&new.edit, Role::Edit),
            (&new.comment, Role::Comment),
            (&new.view, Role::View),
        ] {
            self.db.execute(
                "INSERT INTO tokens (hash, board, can_edit, created, role)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    token_hash(token),
                    new.board,
                    role == Role::Edit,
                    now,
                    role_number(role)
                ],
            )?;
        }
        if let Some(owner) = owner {
            self.db.execute(
                "INSERT INTO members (board, email, role) VALUES (?1, ?2, ?3)",
                params![new.board, owner, OWNER],
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
    pub fn access(&self, board: &str, token: &str) -> rusqlite::Result<Option<Role>> {
        let role: Option<i64> = self
            .db
            .query_row(
                "SELECT role FROM tokens WHERE hash = ?1 AND board = ?2",
                params![token_hash(token), board],
                |row| row.get(0),
            )
            .optional()?;
        Ok(role.map(role_of))
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
        title: Option<&str>,
        assets: &[(AssetId, &[u8])],
    ) -> rusqlite::Result<()> {
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE boards SET snapshot = ?2, updated = ?3, title = ?4 WHERE id = ?1",
            params![board, snapshot, now(), title],
        )?;
        for (id, bytes) in assets {
            tx.execute(
                "INSERT OR IGNORE INTO assets (board, id, bytes) VALUES (?1, ?2, ?3)",
                params![board, id.as_bytes().as_slice(), bytes],
            )?;
        }
        tx.commit()
    }

    /// Starts signing in `email` (an [`email_address`]), unless too many sign-ins for it
    /// are waiting: `None` then.
    pub fn start_login(&self, email: &str) -> rusqlite::Result<Option<Login>> {
        let now = now();
        self.db.execute(
            "DELETE FROM logins WHERE created < ?1",
            params![now - LOGIN_TTL],
        )?;
        let waiting: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM logins WHERE email = ?1",
            params![email],
            |row| row.get(0),
        )?;
        if waiting >= 5 {
            return Ok(None);
        }
        let login = Login {
            request: random_hex(32),
            link: random_hex(32),
            code: login_code(),
        };
        self.db.execute(
            "INSERT INTO logins (request, link, email, code, created) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                token_hash(&login.request),
                token_hash(&login.link),
                email,
                login.code,
                now
            ],
        )?;
        Ok(Some(login))
    }

    /// The address and code of the sign-in an email's `link` is for, while it works.
    pub fn login_for_link(&self, link: &str) -> rusqlite::Result<Option<(String, String)>> {
        self.db
            .query_row(
                "SELECT email, code FROM logins WHERE link = ?1 AND created >= ?2",
                params![token_hash(link), now() - LOGIN_TTL],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
    }

    /// The person followed the email's `link` and said yes: the sign-in is done once the
    /// app asks next. Returns whether the link still worked.
    pub fn confirm_login(&self, link: &str) -> rusqlite::Result<bool> {
        let changed = self.db.execute(
            "UPDATE logins SET confirmed = 1 WHERE link = ?1 AND created >= ?2",
            params![token_hash(link), now() - LOGIN_TTL],
        )?;
        Ok(changed > 0)
    }

    /// The app asks how its sign-in (`request`) goes; once confirmed, the account is made
    /// if new and a session started, and the sign-in is used up.
    pub fn finish_login(&mut self, request: &str) -> rusqlite::Result<LoginState> {
        let now = now();
        let tx = self.db.transaction()?;
        let found: Option<(String, bool)> = tx
            .query_row(
                "SELECT email, confirmed FROM logins WHERE request = ?1 AND created >= ?2",
                params![token_hash(request), now - LOGIN_TTL],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let state = match found {
            None => LoginState::Gone,
            Some((_, false)) => LoginState::Waiting,
            Some((email, true)) => {
                tx.execute(
                    "DELETE FROM logins WHERE request = ?1",
                    params![token_hash(request)],
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO users (email, created) VALUES (?1, ?2)",
                    params![email, now],
                )?;
                let session = random_hex(32);
                tx.execute(
                    "INSERT INTO sessions (hash, email, created, used) VALUES (?1, ?2, ?3, ?3)",
                    params![token_hash(&session), email, now],
                )?;
                LoginState::Done { session, email }
            }
        };
        tx.commit()?;
        Ok(state)
    }

    /// Whose `session` it is, if it is one that works; it counts as used now.
    pub fn session_user(&self, session: &str) -> rusqlite::Result<Option<String>> {
        if session.is_empty() {
            return Ok(None);
        }
        let now = now();
        let hash = token_hash(session);
        let email: Option<String> = self
            .db
            .query_row(
                "SELECT email FROM sessions WHERE hash = ?1 AND used >= ?2",
                params![hash, now - SESSION_TTL],
                |row| row.get(0),
            )
            .optional()?;
        if email.is_some() {
            self.db.execute(
                "UPDATE sessions SET used = ?2 WHERE hash = ?1",
                params![hash, now],
            )?;
        }
        Ok(email)
    }

    /// Signs `session` out.
    pub fn end_session(&self, session: &str) -> rusqlite::Result<()> {
        self.db
            .execute(
                "DELETE FROM sessions WHERE hash = ?1",
                params![token_hash(session)],
            )
            .map(drop)
    }

    /// What `email` is on `board`, if anything.
    pub fn membership(&self, board: &str, email: &str) -> rusqlite::Result<Option<Membership>> {
        let role: Option<i64> = self
            .db
            .query_row(
                "SELECT role FROM members WHERE board = ?1 AND email = ?2",
                params![board, email],
                |row| row.get(0),
            )
            .optional()?;
        Ok(role.map(Membership::of))
    }

    /// The boards `email` is a member of, the latest changed first.
    pub fn boards_of(&self, email: &str) -> rusqlite::Result<Vec<BoardEntry>> {
        let mut query = self.db.prepare(
            "SELECT b.id, b.title, m.role, b.updated FROM members m
             JOIN boards b ON b.id = m.board
             WHERE m.email = ?1 ORDER BY b.updated DESC",
        )?;
        query
            .query_map(params![email], |row| {
                Ok(BoardEntry {
                    board: row.get(0)?,
                    title: row.get(1)?,
                    membership: Membership::of(row.get(2)?),
                    updated: row.get(3)?,
                })
            })?
            .collect()
    }

    /// The members of `board`, by address.
    pub fn members(&self, board: &str) -> rusqlite::Result<Vec<(String, Membership)>> {
        let mut query = self
            .db
            .prepare("SELECT email, role FROM members WHERE board = ?1 ORDER BY email")?;
        query
            .query_map(params![board], |row| {
                Ok((row.get(0)?, Membership::of(row.get(1)?)))
            })?
            .collect()
    }

    /// Lets `email` do what `role` allows on `board`, or with `None` no longer anything.
    /// The owner stays the owner.
    pub fn set_member(&self, board: &str, email: &str, role: Option<Role>) -> rusqlite::Result<()> {
        match role {
            Some(role) => self.db.execute(
                "INSERT INTO members (board, email, role) VALUES (?1, ?2, ?3)
                 ON CONFLICT (board, email) DO UPDATE SET role = ?3 WHERE role < ?4",
                params![board, email, role_number(role), OWNER],
            )?,
            None => self.db.execute(
                "DELETE FROM members WHERE board = ?1 AND email = ?2 AND role < ?3",
                params![board, email, OWNER],
            )?,
        };
        Ok(())
    }

    /// A new link token for `role` on `board`; the old links for that role stop working.
    pub fn renew_link(&self, board: &str, role: Role) -> rusqlite::Result<String> {
        let token = random_hex(16);
        self.db.execute(
            "DELETE FROM tokens WHERE board = ?1 AND role = ?2",
            params![board, role_number(role)],
        )?;
        self.db.execute(
            "INSERT INTO tokens (hash, board, can_edit, created, role) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                token_hash(&token),
                board,
                role == Role::Edit,
                now(),
                role_number(role)
            ],
        )?;
        Ok(token)
    }

    /// The name `board` was last saved with.
    pub fn title(&self, board: &str) -> rusqlite::Result<Option<String>> {
        self.db
            .query_row(
                "SELECT title FROM boards WHERE id = ?1",
                params![board],
                |row| row.get(0),
            )
            .optional()
            .map(Option::flatten)
    }

    /// Removes `board` with its pictures, links and members.
    pub fn delete_board(&self, board: &str) -> rusqlite::Result<()> {
        self.db
            .execute("DELETE FROM boards WHERE id = ?1", params![board])
            .map(drop)
    }

    /// Removes the account of `email`: its sessions, its places on others' boards and the
    /// boards it owns, whose ids are returned.
    pub fn delete_account(&mut self, email: &str) -> rusqlite::Result<Vec<String>> {
        let tx = self.db.transaction()?;
        let owned: Vec<String> = tx
            .prepare("SELECT board FROM members WHERE email = ?1 AND role >= ?2")?
            .query_map(params![email, OWNER], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for board in &owned {
            tx.execute("DELETE FROM boards WHERE id = ?1", params![board])?;
        }
        tx.execute("DELETE FROM members WHERE email = ?1", params![email])?;
        tx.execute("DELETE FROM logins WHERE email = ?1", params![email])?;
        tx.execute("DELETE FROM users WHERE email = ?1", params![email])?;
        tx.commit()?;
        Ok(owned)
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
        assert!(migrate_with(&mut db, &MIGRATIONS[..1]).is_err());
        // A failing step leaves the database as it was.
        let mut db = Connection::open_in_memory().unwrap();
        assert!(migrate_with(&mut db, &[MIGRATIONS[0], "CREATE TABLE x (a); nonsense"]).is_err());
        let version = layout(&db).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn links_from_before_roles_keep_their_access() {
        let mut db = Connection::open_in_memory().unwrap();
        migrate_with(&mut db, &MIGRATIONS[..1]).unwrap();
        db.execute_batch(
            "INSERT INTO boards (id, created, updated) VALUES ('b', 1, 1);
             INSERT INTO tokens (hash, board, can_edit, created) VALUES (x'01', 'b', 1, 1);
             INSERT INTO tokens (hash, board, can_edit, created) VALUES (x'02', 'b', 0, 1);",
        )
        .unwrap();
        migrate(&mut db).unwrap();
        let roles: Vec<i64> = db
            .prepare("SELECT role FROM tokens ORDER BY hash")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(roles, [2, 0]);
    }

    #[test]
    fn tokens_give_their_access_to_their_board_only() {
        let store = Store::in_memory().unwrap();
        let a = store.create_board(None).unwrap();
        let b = store.create_board(None).unwrap();
        assert_eq!(store.access(&a.board, &a.edit).unwrap(), Some(Role::Edit));
        assert_eq!(
            store.access(&a.board, &a.comment).unwrap(),
            Some(Role::Comment)
        );
        assert_eq!(store.access(&a.board, &a.view).unwrap(), Some(Role::View));
        assert_eq!(store.access(&b.board, &a.edit).unwrap(), None);
        assert_eq!(store.access(&a.board, "guess").unwrap(), None);
    }

    #[test]
    fn boards_are_saved_and_loaded() {
        let mut store = Store::in_memory().unwrap();
        let new = store.create_board(None).unwrap();
        assert_eq!(store.load(&new.board).unwrap(), Some((None, vec![])));
        let picture = b"png".as_slice();
        store
            .save(
                &new.board,
                b"snapshot",
                None,
                &[(AssetId::of(picture), picture)],
            )
            .unwrap();
        store
            .save(
                &new.board,
                b"snapshot 2",
                Some("Plans"),
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

    #[test]
    fn email_addresses() {
        assert_eq!(
            email_address("  Ann@Example.COM ").as_deref(),
            Some("ann@example.com")
        );
        for bad in [
            "",
            "ann",
            "@example.com",
            "ann@",
            "ann@host",
            "a b@c.d",
            "a@b@c.d",
            "<a@b.c>",
        ] {
            assert_eq!(email_address(bad), None, "{bad}");
        }
    }

    #[test]
    fn signing_in_by_email_link() {
        let mut store = Store::in_memory().unwrap();
        let login = store.start_login("ann@example.com").unwrap().unwrap();
        assert_eq!(login.code.len(), 4);
        assert_eq!(
            store.finish_login(&login.request).unwrap(),
            LoginState::Waiting
        );
        // The link, not the request, confirms; each secret works only for its own part.
        assert_eq!(
            store.login_for_link(&login.link).unwrap(),
            Some(("ann@example.com".to_owned(), login.code.clone()))
        );
        assert_eq!(store.login_for_link(&login.request).unwrap(), None);
        assert!(!store.confirm_login(&login.request).unwrap());
        assert!(store.confirm_login(&login.link).unwrap());
        let LoginState::Done { session, email } = store.finish_login(&login.request).unwrap()
        else {
            panic!("not signed in");
        };
        assert_eq!(email, "ann@example.com");
        // Used up.
        assert_eq!(
            store.finish_login(&login.request).unwrap(),
            LoginState::Gone
        );
        assert_eq!(
            store.session_user(&session).unwrap().as_deref(),
            Some("ann@example.com")
        );
        assert_eq!(store.session_user("").unwrap(), None);
        store.end_session(&session).unwrap();
        assert_eq!(store.session_user(&session).unwrap(), None);
        // Only a few sign-ins may wait for one address.
        for _ in 0..4 {
            assert!(store.start_login("bob@example.com").unwrap().is_some());
        }
        assert!(store.start_login("bob@example.com").unwrap().is_some());
        assert!(store.start_login("bob@example.com").unwrap().is_none());
    }

    #[test]
    fn owners_let_people_in() {
        let mut store = Store::in_memory().unwrap();
        let new = store.create_board(Some("ann@x.org")).unwrap();
        let owner = store.membership(&new.board, "ann@x.org").unwrap().unwrap();
        assert!(owner.owner && owner.role == Role::Edit);
        assert_eq!(owner.name(), "owner");
        store
            .set_member(&new.board, "bob@x.org", Some(Role::Comment))
            .unwrap();
        store
            .set_member(&new.board, "bob@x.org", Some(Role::View))
            .unwrap();
        // The owner cannot be made less, or removed.
        store
            .set_member(&new.board, "ann@x.org", Some(Role::View))
            .unwrap();
        store.set_member(&new.board, "ann@x.org", None).unwrap();
        let members = store.members(&new.board).unwrap();
        let names: Vec<_> = members
            .iter()
            .map(|(e, m)| (e.as_str(), m.name()))
            .collect();
        assert_eq!(names, [("ann@x.org", "owner"), ("bob@x.org", "view")]);
        store.save(&new.board, b"s", Some("Plans"), &[]).unwrap();
        let listed = store.boards_of("bob@x.org").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title.as_deref(), Some("Plans"));
        // A new link replaces the old one for that role only.
        let comment = store.renew_link(&new.board, Role::Comment).unwrap();
        assert_eq!(store.access(&new.board, &new.comment).unwrap(), None);
        assert_eq!(
            store.access(&new.board, &comment).unwrap(),
            Some(Role::Comment)
        );
        assert_eq!(
            store.access(&new.board, &new.edit).unwrap(),
            Some(Role::Edit)
        );
        // Deleting the account deletes the boards it owns.
        let other = store.create_board(Some("bob@x.org")).unwrap();
        let gone = store.delete_account("ann@x.org").unwrap();
        assert_eq!(gone, std::slice::from_ref(&new.board));
        assert_eq!(store.load(&new.board).unwrap(), None);
        assert!(store.load(&other.board).unwrap().is_some());
        assert!(store.boards_of("bob@x.org").unwrap().len() == 1);
    }
}
