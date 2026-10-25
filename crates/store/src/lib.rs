//! See CLAUDE.md for what this crate owns.

#[doc(hidden)]
pub mod bench;

/// For `fuzz/`: what the search index makes of HTML.
#[doc(hidden)]
pub mod fuzz {
    pub use crate::text::{match_query, plain};
}

mod migrate;
mod model;
mod text;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

pub use migrate::LATEST as SCHEMA_VERSION;
pub use model::{
    Arrival, ArticleKey, ArticleRow, DueSource, FetchState, Filter, Folder, NewArticle, NewSource,
    Source, SourceKind, SourceTarget, StoryCandidate, Validators, View, ViewCounts,
};
pub use text::plain;

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

/// The columns [`article_row`] reads, from `article AS a`.
macro_rules! row_sql {
    () => {
        "a.id, a.source_id, a.title, a.url, a.author, a.sort_at, a.read, a.starred,
         COALESCE(a.story_id, a.id),
         (SELECT count(*) FROM article s WHERE s.story_id = a.story_id)"
    };
}

/// Leaves out an article of `a`'s story when a newer one of it passes the
/// same test, `$also`, so a story is one row: its newest article there.
macro_rules! newest_of_story_sql {
    ($also:expr) => {
        concat!(
            "NOT EXISTS (SELECT 1 FROM article b
                         WHERE b.story_id = a.story_id
                           AND (b.sort_at > a.sort_at OR (b.sort_at = a.sort_at AND b.id > a.id))
                           AND ",
            $also,
            ")"
        )
    };
}

/// A row of [`row_sql!`].
fn article_row(row: &rusqlite::Row) -> rusqlite::Result<ArticleRow> {
    Ok(ArticleRow {
        id: row.get(0)?,
        source_id: row.get(1)?,
        title: row.get(2)?,
        url: row.get(3)?,
        author: row.get(4)?,
        sort_at: row.get(5)?,
        read: row.get(6)?,
        starred: row.get(7)?,
        story_id: row.get(8)?,
        story_size: row.get::<_, i64>(9)?.max(1),
    })
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

    /// Replaces a source's settings, as when an MCP server lists its tools.
    pub fn set_source_config(&self, id: i64, config: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET config = ?2 WHERE id = ?1",
            params![id, config],
        )?;
        Ok(())
    }

    /// Has the next refresh reach a source as if it were new, within a
    /// second if the window is open: its settings changed.
    pub fn recheck(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE source SET checked_at = NULL, retry_at = NULL, error = NULL, failures = 0
             WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// Why each source failed its last refresh, for those that did.
    pub fn source_errors(&self) -> Result<HashMap<i64, String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, error FROM source WHERE error IS NOT NULL")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
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
        self.insert_arrived(articles, &[])
    }

    /// [`insert_articles`](Self::insert_articles), the new ones arriving as
    /// `arrivals` says, in the same order.
    pub fn insert_arrived(
        &mut self,
        articles: &[NewArticle],
        arrivals: &[Arrival],
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let inserted = insert_articles(&tx, articles, arrivals)?;
        tx.commit()?;
        Ok(inserted)
    }

    /// Newest first, across every source or within one, every article on
    /// its own.
    pub fn list_articles(&self, source_id: Option<i64>, limit: u32) -> Result<Vec<ArticleRow>> {
        let mut stmt = self.conn.prepare_cached(concat!(
            "SELECT ",
            row_sql!(),
            " FROM article a
             WHERE ?1 IS NULL OR a.source_id = ?1
             ORDER BY a.sort_at DESC, a.id DESC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![source_id, limit], article_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Newest first, for everything, one source or one folder, narrowed to
    /// a view. A story is one row: its newest article the view shows.
    pub fn list_articles_in(
        &self,
        filter: Filter,
        view: View,
        limit: u32,
    ) -> Result<Vec<ArticleRow>> {
        let (source, folder, view) = in_view(filter, view);
        // Unqualified columns in `in_view_sql!` are the innermost table's:
        // `a` outside, `b` in the story test.
        let mut stmt = self.conn.prepare_cached(concat!(
            "SELECT ",
            row_sql!(),
            " FROM article a
             WHERE ",
            in_view_sql!(),
            " AND ",
            newest_of_story_sql!(in_view_sql!()),
            " ORDER BY a.sort_at DESC, a.id DESC
             LIMIT ?4"
        ))?;
        let rows = stmt.query_map(params![source, folder, view, limit], article_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds the words of the reader page at `url`, as HTML, to the search
    /// index of the articles linking there, after their summaries' words.
    /// Indexing a page again replaces what it added. Returns how many
    /// articles link there.
    pub fn index_page(&mut self, url: &str, html: &str) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let articles: Vec<(i64, String, Option<String>, Option<String>)> = tx
            .prepare("SELECT id, title, author, summary_html FROM article WHERE url = ?1")?
            .query_map([url], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let page = text::plain(html);
        {
            let mut forget = tx.prepare_cached("DELETE FROM article_fts WHERE rowid = ?1")?;
            let mut index = tx.prepare_cached(text::INDEX_ARTICLE)?;
            for (id, title, author, summary) in &articles {
                let summary = summary.as_deref().map(text::plain).unwrap_or_default();
                forget.execute([id])?;
                index.execute(params![id, title, author, format!("{summary} {page}")])?;
            }
        }
        tx.commit()?;
        Ok(articles.len())
    }

    /// Articles whose title, author, summary or reader page (once read)
    /// hold every word typed (each
    /// as a prefix), newest first, a story as one row. Nothing searchable
    /// typed: no results. Any input is safe; it never reaches FTS5 as
    /// syntax.
    pub fn search(&self, typed: &str, limit: u32) -> Result<Vec<ArticleRow>> {
        let Some(query) = text::match_query(typed) else {
            return Ok(Vec::new());
        };
        // The matches go into a set first, then the time index is walked
        // newest first until `limit` are found: a common word stops early
        // instead of sorting every article that has it.
        let mut stmt = self.conn.prepare_cached(concat!(
            "SELECT ",
            row_sql!(),
            " FROM article a INDEXED BY article_by_time
             WHERE a.id IN (SELECT rowid FROM article_fts WHERE article_fts MATCH ?1)
               AND ",
            newest_of_story_sql!(
                "b.id IN (SELECT rowid FROM article_fts WHERE article_fts MATCH ?1)"
            ),
            " ORDER BY a.sort_at DESC, a.id DESC
             LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![query, limit], article_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The articles of a story, newest first.
    pub fn story(&self, story_id: i64) -> Result<Vec<ArticleRow>> {
        let mut stmt = self.conn.prepare_cached(concat!(
            "SELECT ",
            row_sql!(),
            " FROM article a
             WHERE a.story_id = ?1 OR (a.story_id IS NULL AND a.id = ?1)
             ORDER BY a.sort_at DESC, a.id DESC"
        ))?;
        let rows = stmt.query_map([story_id], article_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every article stored since the last grouping, and every article
    /// within `window` seconds of one, oldest first. Empty when all are
    /// grouped.
    pub fn story_candidates(&self, window: i64) -> Result<Vec<StoryCandidate>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, title, sort_at, story_id FROM article
             WHERE sort_at BETWEEN
                     (SELECT min(sort_at) FROM article WHERE story_id IS NULL) - ?1
                 AND (SELECT max(sort_at) FROM article WHERE story_id IS NULL) + ?1
             ORDER BY sort_at, id",
        )?;
        let rows = stmt.query_map([window], |row| {
            Ok(StoryCandidate {
                id: row.get(0)?,
                title: row.get(1)?,
                sort_at: row.get(2)?,
                story_id: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Files articles under stories, `(article, story)`, in one
    /// transaction.
    pub fn set_stories(&mut self, stories: &[(i64, i64)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("UPDATE article SET story_id = ?2 WHERE id = ?1")?;
            for (article, story) in stories {
                stmt.execute([article, story])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Unread stories per source; sources with none are absent.
    pub fn unread_counts(&self) -> Result<HashMap<i64, i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT source_id, count(DISTINCT COALESCE(story_id, id)) FROM article
             WHERE read = 0 GROUP BY source_id",
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

    /// Marks every article of a story read or unread.
    pub fn set_story_read(&self, story_id: i64, read: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE article SET read = ?2
             WHERE story_id = ?1 OR (story_id IS NULL AND id = ?1)",
            params![story_id, read],
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

    /// Every article of `source_id`, newest first, as what tells them apart.
    pub fn article_keys(&self, source_id: i64) -> Result<Vec<ArticleKey>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, guid, url, title FROM article WHERE source_id = ?1
             ORDER BY sort_at DESC, id DESC",
        )?;
        let keys = stmt
            .query_map([source_id], |row| {
                Ok(ArticleKey {
                    id: row.get(0)?,
                    guid: row.get(1)?,
                    url: row.get(2)?,
                    title: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }

    /// Points an article at another link, as when a keyword alert learns
    /// where a Google News link leads.
    pub fn set_article_url(&self, article_id: i64, url: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE article SET url = ?2 WHERE id = ?1",
            params![article_id, url],
        )?;
        Ok(())
    }

    /// Marks every unread article the filter and view show as read, and
    /// the rest of their stories; returns how many.
    pub fn mark_read_in(&self, filter: Filter, view: View) -> Result<usize> {
        let (source, folder, view) = in_view(filter, view);
        Ok(self.conn.execute(
            concat!(
                "UPDATE article SET read = 1 WHERE read = 0 AND (",
                in_view_sql!(),
                " OR story_id IN (SELECT story_id FROM article WHERE story_id IS NOT NULL AND ",
                in_view_sql!(),
                "))"
            ),
            params![source, folder, view],
        )?)
    }

    /// Unread, starred and today's stories across every feed.
    pub fn view_counts(&self) -> Result<ViewCounts> {
        Ok(self.conn.query_row(
            concat!(
                "SELECT
                   count(DISTINCT COALESCE(story_id, id)) FILTER (WHERE read = 0),
                   count(DISTINCT COALESCE(story_id, id)) FILTER (WHERE starred = 1),
                   count(DISTINCT COALESCE(story_id, id)) FILTER (WHERE sort_at >= ",
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

    /// Feeds, keyword alerts and MCP servers whose backoff, if any, has
    /// passed by `now`.
    pub fn sources_due(&self, now: i64) -> Result<Vec<DueSource>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, kind, url, config, etag, last_modified, failures FROM source
             WHERE ((kind = 'feed' AND url IS NOT NULL) OR kind IN ('keyword', 'mcp'))
               AND (retry_at IS NULL OR retry_at <= ?1)
             ORDER BY position",
        )?;
        let rows = stmt.query_map([now], |row| {
            let kind: String = row.get(1)?;
            Ok(DueSource {
                source_id: row.get(0)?,
                kind: SourceKind::parse(&kind).unwrap_or(SourceKind::Feed),
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
                           WHERE ((kind = 'feed' AND url IS NOT NULL) OR kind IN ('keyword', 'mcp'))
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
        self.record_arrived(source_id, articles, &[], validators, now)
    }

    /// [`record_fetched`](Self::record_fetched), the articles new to the
    /// store arriving as `arrivals` says, in the same order (one missing is
    /// unread and unstarred). Articles already stored keep their flags.
    pub fn record_arrived(
        &mut self,
        source_id: i64,
        articles: &[NewArticle],
        arrivals: &[Arrival],
        validators: &Validators,
        now: i64,
    ) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let inserted = insert_articles(&tx, articles, arrivals)?;
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

fn insert_articles(
    tx: &rusqlite::Transaction,
    articles: &[NewArticle],
    arrivals: &[Arrival],
) -> Result<usize> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO article
           (source_id, guid, url, title, author, summary_html,
            published_at, fetched_at, sort_at, read, starred)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (source_id, guid) DO NOTHING",
    )?;
    let mut index = tx.prepare_cached(text::INDEX_ARTICLE)?;
    let mut inserted = 0;
    for (i, a) in articles.iter().enumerate() {
        let arrival = arrivals.get(i).copied().unwrap_or_default();
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
            arrival.read,
            arrival.starred,
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
