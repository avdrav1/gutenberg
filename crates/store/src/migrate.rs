use rusqlite::Connection;

use crate::{Error, Result};

/// Applied in order; `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

pub const LATEST: i64 = MIGRATIONS.len() as i64;

pub fn version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |row| row.get(0))?)
}

/// Brings the schema up to date. Each migration and its version bump commit
/// together, so a failure leaves the file at the last good version.
pub fn run(conn: &mut Connection) -> Result<()> {
    let found = version(conn)?;
    if found > LATEST {
        return Err(Error::TooNew {
            found,
            known: LATEST,
        });
    }
    for (done, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", done as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
