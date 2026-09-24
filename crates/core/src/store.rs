//! The SQLite store at `<git common dir>/stack/stack.db`: the op log, which doubles as the crash-recovery journal.
//!
//! Every mutation is recorded as `pending` with its ref updates before the refs change, then marked `done` (or
//! `failed`). A `pending` operation found later was interrupted.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::git::{Checkout, RefUpdate};
use crate::{Error, MarkKind, OperationKind, OperationState, ReviewMark};

const SCHEMA_VERSION: i64 = 5;

const MIGRATIONS: [&str; 5] = [
    r#"
    CREATE TABLE operation (
        id          INTEGER PRIMARY KEY,
        kind        TEXT    NOT NULL CHECK (kind IN ('command', 'undo', 'redo')),
        description TEXT    NOT NULL,
        target      INTEGER REFERENCES operation (id),
        state       TEXT    NOT NULL CHECK (state IN ('pending', 'done', 'failed')),
        undone      INTEGER NOT NULL DEFAULT 0,
        started_at  INTEGER NOT NULL,
        finished_at INTEGER
    );
    CREATE TABLE ref_update (
        operation INTEGER NOT NULL REFERENCES operation (id),
        name      TEXT    NOT NULL,
        old       TEXT,
        new       TEXT,
        PRIMARY KEY (operation, name)
    );
"#,
    r#"
    ALTER TABLE operation ADD COLUMN checkout_from TEXT;
    ALTER TABLE operation ADD COLUMN checkout_to TEXT;
"#,
    r#"
    CREATE TABLE review_mark (
        key       TEXT    NOT NULL,
        kind      TEXT    NOT NULL CHECK (kind IN ('reviewed', 'tested', 'flagged')),
        note      TEXT,
        marked_at INTEGER NOT NULL,
        PRIMARY KEY (key, kind)
    );
"#,
    r#"
    ALTER TABLE operation ADD COLUMN checkout_worktree TEXT;
    CREATE TABLE follower (
        worktree TEXT PRIMARY KEY,
        branch   TEXT NOT NULL
    );
"#,
    r#"
    CREATE TABLE state (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#,
];

/// An operation as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub id: i64,
    pub kind: OperationKind,
    pub description: String,
    pub target: Option<i64>,
    pub state: OperationState,
    pub undone: bool,
    pub started_at: i64,
    pub updates: Vec<RefUpdate>,
    /// The worktree moved before the refs, if a checked-out branch moved.
    pub checkout: Option<Checkout>,
}

pub(crate) struct Store {
    connection: Connection,
}

impl Store {
    /// Opens the store at `path`, creating it (and its directory) if needed, and migrates it to the current schema.
    ///
    /// # Errors
    /// [`Error::Store`], including for a store written by a newer `stack`.
    pub(crate) fn open(path: &Path) -> Result<Self, Error> {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory).map_err(Error::store)?;
        }
        let connection = Connection::open(path).map_err(Error::store)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(Error::store)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(Error::store)?;
        let store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), Error> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(Error::store)?;
        if version > SCHEMA_VERSION {
            return Err(Error::store(format!(
                "schema version {version} is newer than this stack supports"
            )));
        }
        for (index, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let next = index as i64 + 1;
            let batch = format!("BEGIN; {migration} PRAGMA user_version = {next}; COMMIT;");
            self.connection
                .execute_batch(&batch)
                .map_err(Error::store)?;
        }
        Ok(())
    }

    /// Records a `pending` operation and returns its id.
    pub(crate) fn begin(
        &mut self,
        kind: OperationKind,
        description: &str,
        target: Option<i64>,
        updates: &[RefUpdate],
        checkout: Option<&Checkout>,
    ) -> Result<i64, Error> {
        let transaction = self.connection.transaction().map_err(Error::store)?;
        transaction
            .execute(
                "INSERT INTO operation
                     (kind, description, target, state, started_at, checkout_from, checkout_to, checkout_worktree)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7)",
                params![
                    kind_name(kind),
                    description,
                    target,
                    now(),
                    checkout.map(|checkout| &checkout.from),
                    checkout.map(|checkout| &checkout.to),
                    checkout.and_then(|checkout| checkout.worktree.as_ref()).map(|path| path.to_string_lossy())
                ],
            )
            .map_err(Error::store)?;
        let id = transaction.last_insert_rowid();
        for update in updates {
            transaction
                .execute(
                    "INSERT INTO ref_update (operation, name, old, new) VALUES (?1, ?2, ?3, ?4)",
                    params![id, update.name, update.old, update.new],
                )
                .map_err(Error::store)?;
        }
        transaction.commit().map_err(Error::store)?;
        Ok(id)
    }

    /// Marks operation `id` as `state`. A finished undo or redo also flips its target's `undone` flag.
    pub(crate) fn finish(&mut self, id: i64, state: OperationState) -> Result<(), Error> {
        let transaction = self.connection.transaction().map_err(Error::store)?;
        transaction
            .execute(
                "UPDATE operation SET state = ?2, finished_at = ?3 WHERE id = ?1",
                params![id, state_name(state), now()],
            )
            .map_err(Error::store)?;
        if state == OperationState::Done {
            transaction
                .execute(
                    "UPDATE operation SET undone = (SELECT kind = 'undo' FROM operation WHERE id = ?1)
                     WHERE id = (SELECT target FROM operation WHERE id = ?1)",
                    params![id],
                )
                .map_err(Error::store)?;
        }
        transaction.commit().map_err(Error::store)
    }

    pub(crate) fn pending(&self) -> Result<Vec<Record>, Error> {
        self.query("WHERE state = 'pending' ORDER BY id", params![])
    }

    /// The most recent operations, newest first.
    pub(crate) fn recent(&self, limit: usize) -> Result<Vec<Record>, Error> {
        self.query("ORDER BY id DESC LIMIT ?1", params![limit as i64])
    }

    /// The latest completed command that isn't undone.
    pub(crate) fn undo_target(&self) -> Result<Option<Record>, Error> {
        let found = self.query(
            "WHERE kind = 'command' AND state = 'done' AND undone = 0 ORDER BY id DESC LIMIT 1",
            params![],
        )?;
        Ok(found.into_iter().next())
    }

    /// The most recently undone command, if no command has completed since it was undone.
    pub(crate) fn redo_target(&self) -> Result<Option<Record>, Error> {
        let commands = self.query(
            "WHERE kind = 'command' AND state = 'done' ORDER BY id DESC",
            params![],
        )?;
        // Commands are undone newest first, so the most recently undone is the oldest of the trailing undone run.
        Ok(commands
            .into_iter()
            .take_while(|command| command.undone)
            .last())
    }

    fn query(&self, clause: &str, parameters: impl rusqlite::Params) -> Result<Vec<Record>, Error> {
        let sql = format!(
            "SELECT id, kind, description, target, state, undone, started_at, checkout_from, checkout_to,
                    checkout_worktree
             FROM operation {clause}"
        );
        let mut statement = self.connection.prepare(&sql).map_err(Error::store)?;
        let rows = statement
            .query_map(parameters, |row| {
                Ok(Record {
                    id: row.get(0)?,
                    kind: parse_kind(&row.get::<_, String>(1)?),
                    description: row.get(2)?,
                    target: row.get(3)?,
                    state: parse_state(&row.get::<_, String>(4)?),
                    undone: row.get(5)?,
                    started_at: row.get(6)?,
                    updates: Vec::new(),
                    checkout: {
                        let worktree = row
                            .get::<_, Option<String>>(9)?
                            .map(std::path::PathBuf::from);
                        let from_to = row
                            .get::<_, Option<String>>(7)?
                            .zip(row.get::<_, Option<String>>(8)?);
                        from_to.map(|(from, to)| Checkout { worktree, from, to })
                    },
                })
            })
            .map_err(Error::store)?;
        let mut records = rows.collect::<Result<Vec<_>, _>>().map_err(Error::store)?;
        for record in &mut records {
            record.updates = self.updates(record.id)?;
        }
        Ok(records)
    }

    fn updates(&self, operation: i64) -> Result<Vec<RefUpdate>, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT name, old, new FROM ref_update WHERE operation = ?1 ORDER BY name")
            .map_err(Error::store)?;
        let rows = statement
            .query_map(params![operation], |row| {
                Ok(RefUpdate {
                    name: row.get(0)?,
                    old: row.get(1)?,
                    new: row.get(2)?,
                })
            })
            .map_err(Error::store)?;
        rows.collect::<Result<_, _>>().map_err(Error::store)
    }

    /// Sets (or with `None`, clears) a piece of persistent state, such as a restack waiting on `stack continue`.
    pub(crate) fn set_state(&self, key: &str, value: Option<&str>) -> Result<(), Error> {
        let result = match value {
            Some(value) => self.connection.execute(
                "INSERT OR REPLACE INTO state (key, value) VALUES (?1, ?2)",
                params![key, value],
            ),
            None => self
                .connection
                .execute("DELETE FROM state WHERE key = ?1", params![key]),
        };
        result.map(|_| ()).map_err(Error::store)
    }

    pub(crate) fn state_value(&self, key: &str) -> Result<Option<String>, Error> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT value FROM state WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::store)
    }

    /// Records that the worktree at `worktree` follows `branch`, replacing what it followed before.
    pub(crate) fn set_follower(&self, worktree: &str, branch: &str) -> Result<(), Error> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO follower (worktree, branch) VALUES (?1, ?2)",
                params![worktree, branch],
            )
            .map(|_| ())
            .map_err(Error::store)
    }

    pub(crate) fn remove_follower(&self, worktree: &str) -> Result<(), Error> {
        self.connection
            .execute(
                "DELETE FROM follower WHERE worktree = ?1",
                params![worktree],
            )
            .map(|_| ())
            .map_err(Error::store)
    }

    /// Every `(worktree, branch)` follower, sorted by worktree.
    pub(crate) fn followers(&self) -> Result<Vec<(String, String)>, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT worktree, branch FROM follower ORDER BY worktree")
            .map_err(Error::store)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(Error::store)?;
        rows.collect::<Result<_, _>>().map_err(Error::store)
    }

    /// Sets a review mark on `key`, replacing any of the same kind.
    pub(crate) fn set_mark(
        &self,
        key: &str,
        kind: MarkKind,
        note: Option<&str>,
    ) -> Result<(), Error> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO review_mark (key, kind, note, marked_at) VALUES (?1, ?2, ?3, ?4)",
                params![key, mark_kind_name(kind), note, now()],
            )
            .map(|_| ())
            .map_err(Error::store)
    }

    /// Removes `key`'s marks of `kind`, or all of them. Returns how many were removed.
    pub(crate) fn clear_marks(&self, key: &str, kind: Option<MarkKind>) -> Result<usize, Error> {
        let kind = kind.map(mark_kind_name);
        self.connection
            .execute(
                "DELETE FROM review_mark WHERE key = ?1 AND (?2 IS NULL OR kind = ?2)",
                params![key, kind],
            )
            .map_err(Error::store)
    }

    /// `key`'s marks, ordered by kind.
    pub(crate) fn marks(&self, key: &str) -> Result<Vec<ReviewMark>, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT kind, note, marked_at FROM review_mark WHERE key = ?1 ORDER BY kind")
            .map_err(Error::store)?;
        let rows = statement
            .query_map(params![key], |row| {
                Ok(ReviewMark {
                    kind: parse_mark_kind(&row.get::<_, String>(0)?),
                    note: row.get(1)?,
                    marked_at: row.get(2)?,
                })
            })
            .map_err(Error::store)?;
        rows.collect::<Result<_, _>>().map_err(Error::store)
    }

    #[cfg(test)]
    pub(crate) fn state(&self, id: i64) -> Result<Option<OperationState>, Error> {
        use rusqlite::OptionalExtension;
        self.connection
            .query_row(
                "SELECT state FROM operation WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|state| state.map(|state| parse_state(&state)))
            .map_err(Error::store)
    }
}

fn mark_kind_name(kind: MarkKind) -> &'static str {
    match kind {
        MarkKind::Reviewed => "reviewed",
        MarkKind::Tested => "tested",
        MarkKind::Flagged => "flagged",
    }
}

fn parse_mark_kind(name: &str) -> MarkKind {
    match name {
        "tested" => MarkKind::Tested,
        "flagged" => MarkKind::Flagged,
        _ => MarkKind::Reviewed,
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn kind_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Command => "command",
        OperationKind::Undo => "undo",
        OperationKind::Redo => "redo",
    }
}

fn parse_kind(name: &str) -> OperationKind {
    match name {
        "undo" => OperationKind::Undo,
        "redo" => OperationKind::Redo,
        _ => OperationKind::Command,
    }
}

fn state_name(state: OperationState) -> &'static str {
    match state {
        OperationState::Pending => "pending",
        OperationState::Done => "done",
        OperationState::Failed => "failed",
    }
}

fn parse_state(name: &str) -> OperationState {
    match name {
        "done" => OperationState::Done,
        "failed" => OperationState::Failed,
        _ => OperationState::Pending,
    }
}
