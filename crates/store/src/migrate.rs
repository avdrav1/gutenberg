use rusqlite::{params, Connection, Transaction};

use crate::{text, Error, Result};

/// One schema step: SQL, then optionally Rust for what SQL can't do.
struct Migration {
    sql: &'static str,
    then: Option<fn(&Transaction) -> rusqlite::Result<()>>,
}

/// Applied in order; `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[Migration] = &[
    Migration {
        sql: include_str!("../migrations/0001_init.sql"),
        then: None,
    },
    Migration {
        sql: include_str!("../migrations/0002_fetch_state.sql"),
        then: None,
    },
    Migration {
        sql: include_str!("../migrations/0003_search.sql"),
        then: Some(index_stored_articles),
    },
    Migration {
        sql: include_str!("../migrations/0004_story.sql"),
        then: None,
    },
    Migration {
        sql: include_str!("../migrations/0005_setting.sql"),
        then: None,
    },
];

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
    for (done, migration) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        if let Some(then) = migration.then {
            then(&tx)?;
        }
        tx.pragma_update(None, "user_version", done as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

/// 0003: puts every article stored before search existed in the index.
fn index_stored_articles(tx: &Transaction) -> rusqlite::Result<()> {
    let mut read = tx.prepare("SELECT id, title, author, summary_html FROM article")?;
    let mut write = tx.prepare(text::INDEX_ARTICLE)?;
    let mut rows = read.query([])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let title: String = row.get(1)?;
        let author: Option<String> = row.get(2)?;
        let summary: Option<String> = row.get(3)?;
        write.execute(params![
            id,
            title,
            author,
            summary.as_deref().map(text::plain)
        ])?;
    }
    Ok(())
}
