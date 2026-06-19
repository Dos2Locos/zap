//! A single process-wide `Mutex<SqliteConnection>` shared by the SSH manager.
//!
//! Current state: openWarp's main write connection lives on a dedicated write
//! thread (see `app/src/persistence/sqlite.rs`) and processes work asynchronously
//! through a `ModelEvent` channel. Wiring the SSH manager into that event bus
//! would require 6+ new enum variants plus cross-crate type exposure — too costly.
//!
//! Alternative: **SQLite WAL mode natively supports multiple write connections**
//! (writes are mutually exclusive but retried via `busy_timeout`), so we open a
//! second independent write connection whose behavior stays fully local to this
//! crate. The SSH manager's writes are user-driven (create/delete nodes), happen
//! very rarely, and their contention with the main write thread is negligible.
//!
//! The path is supplied by the caller at init time (`set_database_path`) so this
//! crate does not depend on the app layer's `database_file_path()`. If no path was
//! provided, `with_conn` returns an error.

use anyhow::{Result, anyhow};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DB_PATH: OnceLock<PathBuf> = OnceLock::new();
static CONN: OnceLock<Mutex<SqliteConnection>> = OnceLock::new();

/// Called once at app startup with the sqlite db file path. Repeated calls are
/// ignored (`OnceLock` semantics).
pub fn set_database_path(path: PathBuf) {
    let _ = DB_PATH.set(path);
}

fn open() -> Result<SqliteConnection> {
    let path = DB_PATH
        .get()
        .ok_or_else(|| anyhow!("warp_ssh_manager::db: database path not initialized"))?;
    let url = path.to_string_lossy();
    let mut conn = SqliteConnection::establish(&url)?;
    conn.batch_execute(
        "PRAGMA foreign_keys = ON; \
         PRAGMA busy_timeout = 2000; \
         PRAGMA journal_mode = WAL;",
    )?;
    Ok(conn)
}

/// Run a closure under the connection lock. The connection is opened lazily on
/// first use and reused afterwards.
///
/// A failure to open the database is propagated as an `Err` rather than panicking:
/// the connection is opened behind a fallible path instead of inside
/// `OnceLock::get_or_init` (whose closure cannot return a `Result`).
pub fn with_conn<R>(f: impl FnOnce(&mut SqliteConnection) -> Result<R>) -> Result<R> {
    let mtx = match CONN.get() {
        Some(mtx) => mtx,
        None => {
            // Open outside the lock. If another thread initialized CONN
            // concurrently, our `set` is ignored and we fall back to the winner;
            // the extra connection we opened is simply dropped.
            let conn = open()?;
            let _ = CONN.set(Mutex::new(conn));
            CONN.get()
                .expect("CONN was just set above or by a racing thread")
        }
    };
    let mut guard = mtx
        .lock()
        .map_err(|_| anyhow!("warp_ssh_manager db mutex poisoned"))?;
    f(&mut guard)
}
