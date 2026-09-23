//! The SQLite store at `<git common dir>/stack/stack.db`: the op log, which doubles as the crash-recovery journal.
//!
//! Every mutation is recorded as `pending` with its ref updates before the refs change, then marked `done` (or
//! `failed`). A `pending` operation found later was interrupted.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::git::RefUpdate;
use crate::{Error, OperationKind, OperationState};

const SCHEMA_VERSION: i64 = 2;

const MIGRATIONS: [&str; 2] = [
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
    /// Commits the working tree was moved `(from, to)` before the refs, if the checked-out branch moved.
    pub checkout: Option<(String, String)>,
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
        checkout: Option<&(String, String)>,
    ) -> Result<i64, Error> {
        let transaction = self.connection.transaction().map_err(Error::store)?;
        transaction
            .execute(
                "INSERT INTO operation (kind, description, target, state, started_at, checkout_from, checkout_to)
                 VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6)",
                params![
                    kind_name(kind),
                    description,
                    target,
                    now(),
                    checkout.map(|(from, _)| from),
                    checkout.map(|(_, to)| to)
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
            "SELECT id, kind, description, target, state, undone, started_at, checkout_from, checkout_to
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
                    checkout: row
                        .get::<_, Option<String>>(7)?
                        .zip(row.get::<_, Option<String>>(8)?),
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
