//! See CLAUDE.md for what this crate owns.

#[doc(hidden)]
pub mod bench;
mod migrate;
mod model;
mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

pub use migrate::LATEST as SCHEMA_VERSION;
pub use model::{
    ArticleRow, DueSource, FetchState, Filter, Folder, NewArticle, NewSource, Source, SourceKind,
    SourceTarget, Validators, View, ViewCounts,
};

/// The articles a filter and view pick, as SQL over `article`: `?1` a
/// source, `?2` a folder, `?3` the view's code. A macro, so the queries
/// that use it are built at compile time.
macro_rules! in_view_sql {
    () => {
        concat!(
            "(?1 IS NULL OR source_id = ?1)
             AND (?2 IS NULL OR source_id IN (SELECT id FROM source WHERE folder_id = ?2))
             AND (?3 <> 1 OR read = 0)
             AND (?3 <> 2 OR starred = 1)
             AND (?3 <> 3 OR sort_at >= ",
            today_sql!(),
            ")"
        )
    };
}

/// When today began: local midnight, from the system's time zone.
macro_rules! today_sql {
    () => {
        "unixepoch('now', 'localtime', 'start of day', 'utc')"
    };
}

/// `in_view_sql!`'s parameters for a filter and view.
fn in_view(filter: Filter, view: View) -> (Option<i64>, Option<i64>, i64) {
    let (source, folder) = match filter {
        Filter::All => (None, None),
        Filter::Source(id) => (Some(id), None),
        Filter::Folder(id) => (None, Some(id)),
    };
    let code = match view {
        View::All => 0,
        View::Unread => 1,
        View::Starred => 2,
        View::Today => 3,
    };
    (source, folder, code)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "the database is at schema version {found}, but this build knows only up to {known}; \
         it was written by a newer gutenberg"
    )]
    TooNew { found: i64, known: i64 },
    #[error("no data directory: neither $XDG_DATA_HOME nor $HOME is set")]
    NoDataDir,
    #[error("create {}: {source}", path.display())]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("unknown source kind {0:?} in the database")]
    UnknownKind(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The one SQLite file that holds folders, sources and articles.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// `$XDG_DATA_HOME/gutenberg/gutenberg.db`.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("", "", "gutenberg").ok_or(Error::NoDataDir)?;
        Ok(dirs.data_dir().join("gutenberg.db"))
    }

    /// Opens or creates the file and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| Error::CreateDir {
                path: dir.to_owned(),
                source,
            })?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate::run(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<i64> {
        migrate::version(&self.conn)
    }

    /// Changes when another connection commits, such as the background
    /// refresh or `gutenberg --add` in a terminal. Poll it to know when to
    /// reload; this connection's own writes don't change it.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    pub fn add_folder(&self, name: &str) -> Result<Folder> {
        let position = self.next_position("folder")?;
        self.conn.execute(
            "INSERT INTO folder (name, position) VALUES (?1, ?2)",
            params![name, position],
        )?;
        Ok(Folder {
            id: self.conn.last_insert_rowid(),
            name: name.to_owned(),
            position,
        })
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, name, position FROM folder ORDER BY position")?;
        let rows = stmt.query_map([], |row| {
            Ok(Folder {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_folder(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM folder WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_source(&self, new: &NewSource) -> Result<Source> {
        let position = self.next_position("source")?;
        let created_at = now();
        self.conn.execute(
            "INSERT INTO source (folder_id, kind, title, url, config, position, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new.folder_id,
                new.kind.as_str(),
                new.title,
                new.url,
                new.config,
                position,
                created_at
            ],
        )?;
        Ok(Source {
            id: self.conn.last_insert_rowid(),
            folder_id: new.folder_id,
            kind: new.kind,
            title: new.title.clone(),
            url: new.url.clone(),
            config: new.config.clone(),
            position,
            created_at,
        })
    }

    pub fn sources(&self) -> Result<Vec<Source>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, folder_id, kind, title, url, config, position, created_at
             FROM source ORDER BY position",
        )?;
        let mut rows = stmt.query([])?;
        let mut sources = Vec::new();
        while let Some(row) = rows.next()? {
            let kind: String = row.get(2)?;
            sources.push(Source {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                kind: SourceKind::parse(&kind).ok_or(Error::UnknownKind(kind))?,
                title: row.get(3)?,
                url: row.get(4)?,
                config: row.get(5)?,
                position: row.get(6)?,
                created_at: row.get(7)?,
            });
        }
        Ok(sources)
    }

    pub fn delete_source(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM source WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Inserts in one transaction and returns how many were new. An article
    /// already stored for its source (same GUID) is left as it is, read and
    /// starred flags included.
    pub fn insert_articles(&mut self, articles: &[NewArticle]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let inserted = insert_articles(&tx, articles)?;
        tx.commit()?;
        Ok(inserted)
    }

    /// Newest first, across every source or within one.
    pub fn list_articles(&self, source_id: Option<i64>, limit: u32) -> Result<Vec<ArticleRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, source_id, title, url, author, sort_at, read, starred
             FROM article
             WHERE ?1 IS NULL OR source_id = ?1
             ORDER BY sort_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![source_id, limit], |row| {
            Ok(ArticleRow {
                id: row.get(0)?,
                source_id: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?,
                sort_at: row.get(5)?,
                read: row.get(6)?,
                starred: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Newest first, for everything, one source or one folder, narrowed to
    /// a view.
    pub fn list_articles_in(
        &self,
        filter: Filter,
        view: View,
        limit: u32,
    ) -> Result<Vec<ArticleRow>> {
        let (source, folder, view) = in_view(filter, view);
        let mut stmt = self.conn.prepare_cached(concat!(
            "SELECT id, source_id, title, url, author, sort_at, read, starred
             FROM article
             WHERE ",
            in_view_sql!(),
            " ORDER BY sort_at DESC, id DESC
             LIMIT ?4"
        ))?;
        let rows = stmt.query_map(params![source, folder, view, limit], |row| {
            Ok(ArticleRow {
                id: row.get(0)?,
                source_id: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?,
                sort_at: row.get(5)?,
                read: row.get(6)?,
                starred: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Articles whose title, author or summary hold every word typed (each
    /// as a prefix), newest first. Nothing searchable typed: no results.
    /// Any input is safe; it never reaches FTS5 as syntax.
    pub fn search(&self, typed: &str, limit: u32) -> Result<Vec<ArticleRow>> {
        let Some(query) = text::match_query(typed) else {
            return Ok(Vec::new());
        };
        // The matches go into a set first, then the time index is walked
        // newest first until `limit` are found: a common word stops early
        // instead of sorting every article that has it.
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, source_id, title, url, author, sort_at, read, starred
             FROM article INDEXED BY article_by_time
             WHERE id IN (SELECT rowid FROM article_fts WHERE article_fts MATCH ?1)
             ORDER BY sort_at DESC, id DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![query, limit], |row| {
            Ok(ArticleRow {
                id: row.get(0)?,
                source_id: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                author: row.get(4)?,
                sort_at: row.get(5)?,
                read: row.get(6)?,
                starred: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Unread articles per source; sources with none are absent.
    pub fn unread_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_id, count(*) FROM article WHERE read = 0 GROUP BY source_id",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The folder with this name, made at the end if there isn't one.
    pub fn folder_named(&self, name: &str) -> Result<Folder> {
        let found = self
            .conn
            .query_row(
                "SELECT id, name, position FROM folder WHERE name = ?1 ORDER BY position LIMIT 1",
                [name],
                |row| {
                    Ok(Folder {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        position: row.get(2)?,
                    })
                },
            )
            .optional()?;
        match found {
            Some(folder) => Ok(folder),
            None => self.add_folder(name),
        }
    }

    /// Moves a source and renumbers every source's position, in one
    /// transaction. Sources keep one global order; a folder shows its own
    /// in that order.
    pub fn move_source(&mut self, id: i64, target: SourceTarget) -> Result<()> {
        let tx = self.conn.transaction()?;
        let order: Vec<(i64, Option<i64>)> = {
            let mut stmt = tx.prepare("SELECT id, folder_id FROM source ORDER BY position, id")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut ids: Vec<i64> = order.iter().map(|&(i, _)| i).filter(|&i| i != id).collect();
        let (folder, at) = match target {
            SourceTarget::Before(other) => {
                let folder = order
                    .iter()
                    .find(|&&(i, _)| i == other)
                    .and_then(|&(_, f)| f);
                (
                    folder,
                    ids.iter().position(|&i| i == other).unwrap_or(ids.len()),
                )
            }
            SourceTarget::EndOf(folder) => {
                let last = order
                    .iter()
                    .filter(|&&(i, f)| f == folder && i != id)
                    .filter_map(|&(i, _)| ids.iter().position(|&x| x == i))
                    .max();
                (folder, last.map_or(ids.len(), |p| p + 1))
            }
        };
        ids.insert(at, id);
        tx.execute(
            "UPDATE source SET folder_id = ?2 WHERE id = ?1",
            params![id, folder],
        )?;
        renumber(&tx, "source", &ids)?;
        tx.commit()?;
        Ok(())
    }

    /// Moves a folder just before another, or to the end with `None`.
    pub fn move_folder(&mut self, id: i64, before: Option<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM folder ORDER BY position, id")?;
            let rows = stmt.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        ids.retain(|&i| i != id);
        let at = before
            .and_then(|b| ids.iter().position(|&i| i == b))
            .unwrap_or(ids.len());
        ids.insert(at, id);
        renumber(&tx, "folder", &ids)?;
        tx.commit()?;
        Ok(())
    }

    pub fn summary(&self, article_id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT summary_html FROM article WHERE id = ?1",
                [article_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_read(&self, article_id: i64, read: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE article SET read = ?2 WHERE id = ?1",
            params![article_id, read],
        )?;
        Ok(())
    }

    pub fn set_starred(&self, article_id: i64, starred: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE article SET starred = ?2 WHERE id = ?1",
            params![article_id, starred],
        )?;
        Ok(())
    }

    /// Marks every unread article the filter and view show as read; returns
    /// how many.
    pub fn mark_read_in(&self, filter: Filter, view: View) -> Result<usize> {
        let (source, folder, view) = in_view(filter, view);
        Ok(self.conn.execute(
            concat!(
                "UPDATE article SET read = 1 WHERE read = 0 AND ",
                in_view_sql!()
            ),
            params![source, folder, view],
        )?)
    }

    /// Unread, starred and today's articles across every feed.
    pub fn view_counts(&self) -> Result<ViewCounts> {
        Ok(self.conn.query_row(
            concat!(
                "SELECT
                   count(*) FILTER (WHERE read = 0),
                   count(*) FILTER (WHERE starred = 1),
                   count(*) FILTER (WHERE sort_at >= ",
                today_sql!(),
                ")
                 FROM article"
            ),
            [],
            |row| {
                Ok(ViewCounts {
                    unread: row.get(0)?,
                    starred: row.get(1)?,
                    today: row.get(2)?,
                })
            },
        )?)
    }

    /// Feeds and keyword sources whose backoff, if any, has passed by `now`.
    pub fn sources_due(&self, now: i64) -> Result<Vec<DueSource>> {
        // MCP sources (E6) aren't refreshed yet.
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, kind, url, config, etag, last_modified, failures FROM source
             WHERE ((kind = 'feed' AND url IS NOT NULL) OR kind = 'keyword')
               AND (retry_at IS NULL OR retry_at <= ?1)
             ORDER BY position",
        )?;
        let rows = stmt.query_map([now], |row| {
            let kind: String = row.get(1)?;
            Ok(DueSource {
                source_id: row.get(0)?,
                kind: if kind == "keyword" {
                    SourceKind::Keyword
                } else {
                    SourceKind::Feed
                },
                url: row.get(2)?,
                config: row.get(3)?,
                validators: Validators {
                    etag: row.get(4)?,
                    last_modified: row.get(5)?,
                },
                failures: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Whether a source has never been fetched: added since the last refresh
    /// (the Add sources screen, `--add`, `--keyword`).
    pub fn has_unchecked_sources(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM source
                           WHERE ((kind = 'feed' AND url IS NOT NULL) OR kind = 'keyword')
                             AND checked_at IS NULL)",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn fetch_state(&self, source_id: i64) -> Result<FetchState> {
        Ok(self.conn.query_row(
            "SELECT etag, last_modified, checked_at, error, failures, retry_at
             FROM source WHERE id = ?1",
            [source_id],
            |row| {
                Ok(FetchState {
                    validators: Validators {
                        etag: row.get(0)?,
                        last_modified: row.get(1)?,
                    },
                    checked_at: row.get(2)?,
                    error: row.get(3)?,
                    failures: row.get(4)?,
                    retry_at: row.get(5)?,
                })
            },
        )?)
    }

    /// A 200: new articles and the validators to send next time, in one
    /// transaction. Returns how many articles were new.
    pub fn record_fetched(
        &mut self,
        source_id: i64,
        articles: &[NewArticle],
        validators: &Validators,
        now: i64,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let inserted = insert_articles(&tx, articles)?;
        tx.execute(
            "UPDATE source SET etag = ?2, last_modified = ?3, checked_at = ?4,
                    error = NULL, failures = 0, retry_at = NULL
             WHERE id = ?1",
            params![source_id, validators.etag, validators.last_modified, now],
        )?;
        tx.commit()?;
        Ok(inserted)
    }

    /// A 304: nothing new, and the source is healthy.
    pub fn record_not_modified(&self, source_id: i64, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET checked_at = ?2, error = NULL, failures = 0, retry_at = NULL
             WHERE id = ?1",
            params![source_id, now],
        )?;
        Ok(())
    }

    /// A failed refresh: one line of why, and when to try again.
    pub fn record_failure(
        &self,
        source_id: i64,
        error: &str,
        retry_at: i64,
        now: i64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET checked_at = ?2, error = ?3, failures = failures + 1, retry_at = ?4
             WHERE id = ?1",
            params![source_id, now, error, retry_at],
        )?;
        Ok(())
    }

    fn next_position(&self, table: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            &format!("SELECT COALESCE(MAX(position), -1) + 1 FROM {table}"),
            [],
            |row| row.get(0),
        )?)
    }
}

/// Positions 0, 1, 2… in the given order.
fn renumber(tx: &rusqlite::Transaction, table: &str, ids: &[i64]) -> Result<()> {
    let mut stmt = tx.prepare(&format!("UPDATE {table} SET position = ?2 WHERE id = ?1"))?;
    for (position, id) in ids.iter().enumerate() {
        stmt.execute(params![id, position as i64])?;
    }
    Ok(())
}

fn insert_articles(tx: &rusqlite::Transaction, articles: &[NewArticle]) -> Result<usize> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO article
           (source_id, guid, url, title, author, summary_html,
            published_at, fetched_at, sort_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (source_id, guid) DO NOTHING",
    )?;
    let mut index = tx.prepare_cached(text::INDEX_ARTICLE)?;
    let mut inserted = 0;
    for a in articles {
        let added = stmt.execute(params![
            a.source_id,
            a.guid,
            a.url,
            a.title,
            a.author,
            a.summary_html,
            a.published_at,
            a.fetched_at,
            a.published_at.unwrap_or(a.fetched_at),
        ])?;
        if added > 0 {
            index.execute(params![
                tx.last_insert_rowid(),
                a.title,
                a.author,
                a.summary_html.as_deref().map(text::plain),
            ])?;
        }
        inserted += added;
    }
    Ok(inserted)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
