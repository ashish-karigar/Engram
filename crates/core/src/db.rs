use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;
use uuid::Uuid;

use crate::error::Result;

/// Ordered, append-only migrations. Never edit a shipped entry; add a new file.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001.sql"),
    include_str!("../migrations/002.sql"),
    include_str!("../migrations/003.sql"),
    include_str!("../migrations/004.sql"),
    include_str!("../migrations/005.sql"),
    include_str!("../migrations/006.sql"),
    include_str!("../migrations/007.sql"),
    include_str!("../migrations/008.sql"),
];

/// One connection shared across threads. SQLite work here is short, so a mutex is enough.
pub type SharedDb = Arc<Mutex<Connection>>;

pub fn shared(conn: Connection) -> SharedDb {
    Arc::new(Mutex::new(conn))
}

pub fn new_id(prefix: &str) -> String {
    let uuid = Uuid::new_v4().simple().to_string();
    format!("{prefix}_{}", &uuid[..20])
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    configure(Connection::open(path)?)
}

pub fn open_memory() -> Result<Connection> {
    configure(Connection::open_in_memory()?)
}

fn configure(conn: Connection) -> Result<Connection> {
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        conn.execute_batch(&format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1))?;
    }
    Ok(())
}

/// Run `f` inside a savepoint. Savepoints nest, so callers can compose transactional helpers.
pub fn tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT engram")?;
    match f(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE engram")?;
            Ok(value)
        }
        Err(err) => {
            conn.execute_batch("ROLLBACK TO engram; RELEASE engram")?;
            Err(err)
        }
    }
}
