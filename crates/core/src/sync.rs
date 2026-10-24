//! Refresh: fetch the feeds that are due with conditional GET, within
//! overall and per-host limits, and write each result to the store as it
//! arrives. Failures back off; they are never an error the user must
//! dismiss.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::stream::{FuturesUnordered, StreamExt};
use gutenberg_sources::feed::{self, Entry, Feed, FeedError, Fetched};
use gutenberg_sources::image::Format;
use gutenberg_sources::keyword::{self, Keyword, Provider, Providers, Seen};
use gutenberg_sources::mcp::{self, Mcp};
use gutenberg_sources::{favicon, google_news};
use gutenberg_store::{DueSource, NewArticle, SourceKind, Store, Validators};
use tokio::sync::Semaphore;

use crate::config::{self, Config};
use crate::favicon::Icons;

/// How hard a refresh may hit the network.
#[derive(Debug, Clone)]
pub struct Limits {
    /// Fetches in flight at once.
    pub total: usize,
    /// Fetches in flight to one host (`host:port`).
    pub per_host: usize,
    /// Least time between two request starts to one host.
    pub spacing: Duration,
    /// Where keyword alerts search.
    pub providers: Providers,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            total: 16,
            per_host: 2,
            spacing: Duration::from_millis(250),
            providers: Providers::default(),
        }
    }
}

/// Google News links a keyword alert resolves to the publisher's URL per
/// refresh, new results first, then older ones still pointing at Google.
/// Each takes two requests.
pub const RESOLVE_PER_REFRESH: usize = 20;

/// What a refresh did, for `gutenberg --refresh` and logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Feeds and keyword alerts that were due.
    pub sources: usize,
    /// Sources that brought new articles.
    pub updated: usize,
    pub new_articles: usize,
    /// 304s, and 200s with nothing new.
    pub unchanged: usize,
    /// What failed (a feed's URL, a keyword alert's query) and one line of
    /// why.
    pub failed: Vec<(String, String)>,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
        write!(
            f,
            "{}: {} updated ({}), {} unchanged, {} failed",
            plural(self.sources, "source"),
            self.updated,
            plural(self.new_articles, "new article"),
            self.unchanged,
            self.failed.len()
        )
    }
}

/// First retry after a failure; it doubles with each failure in a row.
const BACKOFF_BASE: i64 = 5 * 60;
/// Longest wait, whatever `Retry-After` says.
const BACKOFF_MAX: i64 = 6 * 60 * 60;

/// Seconds to wait after the `failures`-th failure in a row.
pub fn backoff(failures: i64, retry_after: Option<u64>) -> i64 {
    let doubling = BACKOFF_BASE.saturating_mul(1 << (failures - 1).clamp(0, 16));
    retry_after
        .map_or(doubling, |s| i64::try_from(s).unwrap_or(BACKOFF_MAX))
        .min(BACKOFF_MAX)
}

/// A due source, resolved to what to fetch.
struct Job {
    source_id: i64,
    /// What `--refresh` calls it: a feed's URL, a keyword alert's query.
    label: String,
    url: String,
    work: Work,
    validators: Validators,
    failures: i64,
}

/// What refreshing a source means.
enum Work {
    Feed,
    Keyword(KeywordJob),
    /// Reach the server and list its tools.
    Mcp(Mcp),
}

/// What a keyword alert's refresh needs from the store.
struct KeywordJob {
    keyword: Keyword,
    /// Every article it has.
    seen: Seen,
    /// Its articles still on a Google News link, newest first, at most
    /// [`RESOLVE_PER_REFRESH`].
    unresolved: Vec<(i64, String)>,
}

/// `(url, label, work)` for a due source. A keyword source's URL is its
/// Google News search, an MCP server's its URL or command line; settings
/// that can't be read are its failure.
fn resolve(
    store: &Store,
    due: &DueSource,
    providers: &Providers,
) -> Result<(String, String, Work), String> {
    match (due.kind, &due.url) {
        (SourceKind::Keyword, _) => {
            let keyword = Keyword::from_config(&due.config).map_err(|e| e.to_string())?;
            let mut seen = Seen::default();
            let mut unresolved = Vec::new();
            for key in store
                .article_keys(due.source_id)
                .map_err(|e| e.to_string())?
            {
                seen.insert(&key.guid, key.url.as_deref(), &key.title);
                match key.url {
                    Some(url)
                        if unresolved.len() < RESOLVE_PER_REFRESH
                            && google_news::is_article_link(&url, &providers.google_news) =>
                    {
                        unresolved.push((key.id, url));
                    }
                    _ => {}
                }
            }
            Ok((
                keyword
                    .search_url(Provider::GoogleNews, providers)
                    .expect("Google News reads every query"),
                format!("keyword alert {}", keyword.query),
                Work::Keyword(KeywordJob {
                    keyword,
                    seen,
                    unresolved,
                }),
            ))
        }
        (SourceKind::Mcp, _) => {
            let mcp = Mcp::from_config(&due.config).map_err(|e| e.to_string())?;
            let label = mcp.server.label();
            Ok((label.clone(), format!("MCP server {label}"), Work::Mcp(mcp)))
        }
        (_, Some(url)) => Ok((url.clone(), url.clone(), Work::Feed)),
        (_, None) => Err("no URL".to_owned()),
    }
}

/// Refreshes every feed, keyword alert and MCP server due at `now`. Store errors stop
/// the refresh; a source's own failure is recorded on it and the refresh
/// goes on.
///
/// With `icons`, a feed with nothing in the cache is fetched in full once,
/// for its site link, and the site's icon is saved before the feed's
/// articles are, so a reader of the store finds it. After that the site
/// isn't asked again. Keyword alerts have no site of their own.
pub async fn refresh(
    store: &mut Store,
    now: i64,
    limits: &Limits,
    icons: Option<&Icons>,
) -> Result<Summary, gutenberg_store::Error> {
    refresh_some(store, now, limits, icons, |_| true).await
}

/// Refreshes one source now, if it's due, as when it's just been added.
pub async fn refresh_source(
    store: &mut Store,
    source_id: i64,
    now: i64,
    limits: &Limits,
) -> Result<Summary, gutenberg_store::Error> {
    refresh_some(store, now, limits, None, |due| due.source_id == source_id).await
}

async fn refresh_some(
    store: &mut Store,
    now: i64,
    limits: &Limits,
    icons: Option<&Icons>,
    pick: impl Fn(&DueSource) -> bool,
) -> Result<Summary, gutenberg_store::Error> {
    let mut due = store.sources_due(now)?;
    due.retain(|d| pick(d));
    let settled = icons.map(Icons::settled).unwrap_or_default();
    let mut summary = Summary {
        sources: due.len(),
        ..Summary::default()
    };
    let mut jobs = Vec::with_capacity(due.len());
    for due in due {
        match resolve(store, &due, &limits.providers) {
            Ok((url, label, work)) => jobs.push(Job {
                source_id: due.source_id,
                label,
                url,
                work,
                validators: due.validators,
                failures: due.failures,
            }),
            Err(error) => {
                let wait = backoff(due.failures + 1, None);
                store.record_failure(due.source_id, &error, now + wait, now)?;
                summary
                    .failed
                    .push((format!("source {}", due.source_id), error));
            }
        }
    }
    let client = match feed::client() {
        Ok(client) => client,
        Err(err) => {
            for job in jobs {
                summary.failed.push((job.label, err.to_string()));
            }
            return Ok(summary);
        }
    };
    let hosts = Hosts::new(limits);
    // Every source waits at once: one stuck behind its host's limit must not
    // hold a slot that another host could use. `Hosts` caps what's sent.
    let mut results: FuturesUnordered<_> = jobs
        .into_iter()
        .map(|mut due| {
            let (client, hosts) = (&client, &hosts);
            let icons = icons.filter(|_| {
                matches!(due.work, Work::Feed) && !settled.contains(&Icons::key(&due.url))
            });
            async move {
                let outcome = match std::mem::replace(&mut due.work, Work::Feed) {
                    Work::Keyword(job) => {
                        let fetch = Fetch {
                            client,
                            hosts,
                            providers: &limits.providers,
                        };
                        fetch_keyword(&fetch, job, due.source_id, now).await
                    }
                    Work::Mcp(server) => list_tools(server).await,
                    Work::Feed => fetch_one(client, hosts, &due, now, icons.is_some()).await,
                };
                if let (Some(icons), Outcome::Fetched { site_url, .. }) = (icons, &outcome) {
                    let page = site_url.clone().or_else(|| site_root(&due.url));
                    if let Some(page) = page {
                        save_icon(icons, &due.url, fetch_icon(client, hosts, &page).await);
                    }
                }
                (due, outcome)
            }
        })
        .collect();

    while let Some((due, outcome)) = results.next().await {
        match outcome {
            Outcome::NotModified => {
                store.record_not_modified(due.source_id, now)?;
                summary.unchanged += 1;
            }
            Outcome::Fetched {
                articles,
                validators,
                moved,
                ..
            } => {
                let new = store.record_fetched(due.source_id, &articles, &validators, now)?;
                for (article_id, url) in moved {
                    store.set_article_url(article_id, &url)?;
                }
                if new > 0 {
                    summary.updated += 1;
                    summary.new_articles += new;
                } else {
                    summary.unchanged += 1;
                }
            }
            Outcome::Listed { config } => {
                store.set_source_config(due.source_id, &config)?;
                store.record_not_modified(due.source_id, now)?;
                summary.unchanged += 1;
            }
            Outcome::Failed { error, retry_after } => {
                let wait = backoff(due.failures + 1, retry_after);
                store.record_failure(due.source_id, &error, now + wait, now)?;
                summary.failed.push((due.label, error));
            }
        }
    }
    crate::story::group_stored(store)?;
    Ok(summary)
}

/// A feed's entries as rows for the store.
pub fn articles(feed: &Feed, source_id: i64, fetched_at: i64) -> Vec<NewArticle> {
    rows(&feed.entries, source_id, fetched_at)
}

fn rows(entries: &[Entry], source_id: i64, fetched_at: i64) -> Vec<NewArticle> {
    entries
        .iter()
        .map(|e| NewArticle {
            source_id,
            guid: e.guid.clone(),
            url: e.url.clone(),
            title: e.title.clone(),
            author: e.author.clone(),
            summary_html: e.summary_html.clone(),
            published_at: e.published_at,
            fetched_at,
        })
        .collect()
}

enum Outcome {
    NotModified,
    Fetched {
        articles: Vec<NewArticle>,
        validators: Validators,
        /// The page the feed says it belongs to.
        site_url: Option<String>,
        /// Articles already stored whose link now goes to the publisher.
        moved: Vec<(i64, String)>,
    },
    /// An MCP server answered; its settings with the tools it listed.
    Listed {
        config: String,
    },
    Failed {
        error: String,
        retry_after: Option<u64>,
    },
}

/// Reaches an MCP server and lists its tools.
async fn list_tools(mut server: Mcp) -> Outcome {
    match mcp::tools(&server.server, mcp::TIMEOUT).await {
        Ok(tools) => {
            server.tools = tools;
            Outcome::Listed {
                config: server.to_config(),
            }
        }
        Err(e) => Outcome::Failed {
            error: e.to_string(),
            retry_after: None,
        },
    }
}

/// `fresh` drops the validators, so the server sends the feed in full.
async fn fetch_one(
    client: &feed::Client,
    hosts: &Hosts,
    due: &Job,
    now: i64,
    fresh: bool,
) -> Outcome {
    let (etag, last_modified) = if fresh {
        (None, None)
    } else {
        (
            due.validators.etag.as_deref(),
            due.validators.last_modified.as_deref(),
        )
    };
    let fetched = get(client, hosts, &due.url, etag, last_modified).await;
    match fetched {
        Ok(Fetched::NotModified) => Outcome::NotModified,
        Ok(Fetched::Body {
            bytes,
            etag,
            last_modified,
        }) => match feed::parse(&bytes, &due.url) {
            Ok(parsed) => Outcome::Fetched {
                articles: articles(&parsed, due.source_id, now),
                validators: Validators {
                    etag,
                    last_modified,
                },
                site_url: parsed.site_url,
                moved: Vec::new(),
            },
            Err(err) => Outcome::Failed {
                error: err.to_string(),
                retry_after: None,
            },
        },
        Err(err) => Outcome::Failed {
            retry_after: retry_after(&err),
            error: err.to_string(),
        },
    }
}

/// What a keyword alert's refresh sends requests with.
struct Fetch<'a> {
    client: &'a feed::Client,
    hosts: &'a Hosts,
    providers: &'a Providers,
}

/// Asks every provider, merges what they found with what the alert has,
/// and resolves Google News links. One provider failing isn't the alert
/// failing; all of them is.
async fn fetch_keyword(fetch: &Fetch<'_>, job: KeywordJob, source_id: i64, now: i64) -> Outcome {
    let KeywordJob {
        keyword,
        mut seen,
        unresolved,
    } = job;
    let keyword = &keyword;
    // A provider that can't ask the query sits it out.
    let searches = Provider::ALL.into_iter().filter_map(|provider| {
        let url = keyword.search_url(provider, fetch.providers)?;
        Some(async move {
            match get(fetch.client, fetch.hosts, &url, None, None).await {
                Ok(Fetched::Body { bytes, .. }) => {
                    let mut parsed =
                        feed::parse(&bytes, &url).map_err(|e| (e.to_string(), None))?;
                    keyword::tidy(provider, &mut parsed, &bytes);
                    parsed.entries.retain(|e| !keyword.excludes(e));
                    Ok(parsed.entries)
                }
                Ok(Fetched::NotModified) => Ok(Vec::new()),
                Err(err) => Err((err.to_string(), retry_after(&err))),
            }
        })
    });
    let mut found = Vec::new();
    let mut failure = None;
    for result in futures::future::join_all(searches).await {
        match result {
            Ok(entries) => found.push(entries),
            Err(err) => failure = failure.or(Some(err)),
        }
    }
    if found.is_empty() {
        let (error, retry_after) = failure.unwrap_or_else(|| ("no provider".to_owned(), None));
        return Outcome::Failed { error, retry_after };
    }

    let mut entries = keyword::merge(found, &mut seen);
    let google = |url: &str| google_news::is_article_link(url, &fetch.providers.google_news);
    let new: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].url.as_deref().is_some_and(google))
        .take(RESOLVE_PER_REFRESH)
        .collect();
    let old = unresolved.into_iter().take(RESOLVE_PER_REFRESH - new.len());
    let links: Vec<String> = new
        .iter()
        .filter_map(|&i| entries[i].url.clone())
        .chain(old.clone().map(|(_, url)| url))
        .collect();
    let resolved =
        futures::future::join_all(links.iter().map(|link| resolve_google(fetch, link))).await;
    let (resolved_new, resolved_old) = resolved.split_at(new.len());

    // A new result that turns out to be a story the alert has is dropped.
    let mut drop = Vec::new();
    for (&i, url) in new.iter().zip(resolved_new) {
        let Some(url) = url else { continue };
        if seen.has_url(url) {
            drop.push(i);
        } else {
            seen.insert(&entries[i].guid, Some(url), "");
            entries[i].url = Some(url.clone());
        }
    }
    for i in drop.into_iter().rev() {
        entries.remove(i);
    }
    let moved = old
        .zip(resolved_old)
        .filter_map(|((id, _), url)| Some((id, url.clone()?)))
        .collect();
    Outcome::Fetched {
        articles: rows(&entries, source_id, now),
        validators: Validators::default(),
        site_url: None,
        moved,
    }
}

/// The publisher's URL for a Google News link, within the host limits;
/// `None` when Google doesn't say.
async fn resolve_google(fetch: &Fetch<'_>, link: &str) -> Option<String> {
    let page = limited(
        fetch.hosts,
        link,
        google_news::fetch_page(fetch.client, link),
    )
    .await
    .ok()?;
    let sig = google_news::signature(&page)?;
    let base = &fetch.providers.google_news;
    limited(
        fetch.hosts,
        base,
        google_news::resolve_signed(fetch.client, base, &sig),
    )
    .await
    .ok()
}

fn retry_after(err: &FeedError) -> Option<u64> {
    match err {
        FeedError::Status { retry_after, .. } => *retry_after,
        _ => None,
    }
}

/// One GET within the overall and per-host limits.
async fn get(
    client: &feed::Client,
    hosts: &Hosts,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Fetched, FeedError> {
    limited(
        hosts,
        url,
        feed::fetch_conditional(client, url, etag, last_modified),
    )
    .await
}

/// Runs `request` to `url`'s host within the overall and per-host limits.
/// The permits are held only while it's in flight, so a feed's icon
/// requests never wait on a permit its own feed request still holds.
async fn limited<T>(hosts: &Hosts, url: &str, request: impl std::future::Future<Output = T>) -> T {
    let host = hosts.get(url);
    let _host = host.in_flight.acquire().await;
    host.wait_turn(hosts.spacing).await;
    let _sending = hosts.sending.acquire().await;
    request.await
}

/// Icon URLs tried per site, best first, before giving up on it.
const ICON_TRIES: usize = 3;

enum IconFetch {
    Found(Vec<u8>, Format),
    /// The site answered and has no icon we can use.
    Missing,
    /// The site didn't answer; ask again next refresh.
    Later,
}

/// The icon `page_url` names, else `/favicon.ico` on its site.
async fn fetch_icon(client: &feed::Client, hosts: &Hosts, page_url: &str) -> IconFetch {
    let page = match get(client, hosts, page_url, None, None).await {
        Ok(Fetched::Body { bytes, .. }) => bytes,
        Ok(Fetched::NotModified) => Vec::new(),
        // A missing page can still leave /favicon.ico; a broken server can't.
        Err(FeedError::Status { status, .. }) if status.is_client_error() => Vec::new(),
        Err(_) => return IconFetch::Later,
    };
    let mut candidates = favicon::icon_links(&String::from_utf8_lossy(&page), page_url);
    if let Some(root) = favicon::root_icon(page_url) {
        if !candidates.contains(&root) {
            candidates.push(root);
        }
    }
    for url in candidates.iter().take(ICON_TRIES) {
        if let Ok(Fetched::Body { bytes, .. }) = get(client, hosts, url, None, None).await {
            if let Some(format) = favicon::sniff(&bytes) {
                return IconFetch::Found(bytes, format);
            }
        }
    }
    IconFetch::Missing
}

fn save_icon(icons: &Icons, feed_url: &str, fetched: IconFetch) {
    let saved = match &fetched {
        IconFetch::Found(bytes, format) => icons.save(feed_url, Some((bytes, *format))),
        IconFetch::Missing => icons.save(feed_url, None),
        IconFetch::Later => return,
    };
    if let Err(err) = saved {
        eprintln!("gutenberg: saving the icon for {feed_url}: {err}");
    }
}

/// The root of the feed's own site, for a feed that names no site.
fn site_root(feed_url: &str) -> Option<String> {
    let url = url::Url::parse(feed_url).ok()?.join("/").ok()?;
    Some(url.into())
}

/// The overall limit, and per-host limits created on first use.
struct Hosts {
    /// Requests in flight across all hosts.
    sending: Semaphore,
    per_host: usize,
    spacing: Duration,
    hosts: Mutex<HashMap<String, Arc<Host>>>,
}

struct Host {
    in_flight: Semaphore,
    /// When the next request to this host may start.
    next_start: tokio::sync::Mutex<Option<Instant>>,
}

impl Hosts {
    fn new(limits: &Limits) -> Self {
        Self {
            sending: Semaphore::new(limits.total.max(1)),
            per_host: limits.per_host.max(1),
            spacing: limits.spacing,
            hosts: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, url: &str) -> Arc<Host> {
        let key = url::Url::parse(url)
            .ok()
            .and_then(|u| Some(format!("{}:{}", u.host_str()?, u.port_or_known_default()?)))
            .unwrap_or_else(|| url.to_owned());
        let mut hosts = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        hosts
            .entry(key)
            .or_insert_with(|| {
                Arc::new(Host {
                    in_flight: Semaphore::new(self.per_host),
                    next_start: tokio::sync::Mutex::new(None),
                })
            })
            .clone()
    }
}

impl Host {
    async fn wait_turn(&self, spacing: Duration) {
        let mut next = self.next_start.lock().await;
        let now = Instant::now();
        let start = next.map_or(now, |t| t.max(now));
        *next = Some(start + spacing);
        drop(next);
        tokio::time::sleep_until(start.into()).await;
    }
}

/// Refreshes in the background for as long as the app runs: once at
/// start, then every `[refresh] minutes` (0 pauses it). The interval
/// follows `config.toml` edits. Opens its own store connection, so the UI
/// thread never waits on it.
pub fn spawn(store_path: PathBuf, config_path: Option<PathBuf>, config: &Config) {
    let minutes = Arc::new(AtomicU32::new(config.refresh.minutes));
    let watched = minutes.clone();
    let spawned = std::thread::Builder::new()
        .name("refresh".into())
        .spawn(move || {
            let _watch = config_path.and_then(|path| {
                config::watch(&path, move |loaded| {
                    if let Ok(config) = loaded {
                        watched.store(config.refresh.minutes, Ordering::Relaxed);
                    }
                })
                .ok()
            });
            if let Err(err) = run(&store_path, &minutes) {
                eprintln!("gutenberg: background refresh stopped: {err}");
            }
        });
    if let Err(err) = spawned {
        eprintln!("gutenberg: background refresh did not start: {err}");
    }
}

fn run(store_path: &std::path::Path, minutes: &AtomicU32) -> Result<(), String> {
    let mut store = Store::open(store_path).map_err(|e| e.to_string())?;
    let icons = Icons::beside(store_path);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let limits = Limits::default();
    loop {
        if minutes.load(Ordering::Relaxed) > 0 {
            runtime
                .block_on(refresh(&mut store, unix_now(), &limits, Some(&icons)))
                .map_err(|e| e.to_string())?;
        }
        // Wait out the interval, re-reading it so an edit applies now.
        let since = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let every = u64::from(minutes.load(Ordering::Relaxed)) * 60;
            if every > 0 && since.elapsed().as_secs() >= every {
                break;
            }
            // Sources added since the last refresh fetch now.
            if every > 0 && store.has_unchecked_sources().map_err(|e| e.to_string())? {
                break;
            }
        }
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(1, None), 300);
        assert_eq!(backoff(2, None), 600);
        assert_eq!(backoff(3, None), 1200);
        assert_eq!(backoff(20, None), BACKOFF_MAX);
    }

    #[test]
    fn retry_after_wins_but_is_capped() {
        assert_eq!(backoff(1, Some(120)), 120);
        assert_eq!(backoff(5, Some(120)), 120);
        assert_eq!(backoff(1, Some(999_999)), BACKOFF_MAX);
    }

    #[test]
    fn summary_reads_as_a_sentence() {
        let summary = Summary {
            sources: 12,
            updated: 3,
            new_articles: 27,
            unchanged: 8,
            failed: vec![("u".into(), "e".into())],
        };
        assert_eq!(
            summary.to_string(),
            "12 sources: 3 updated (27 new articles), 8 unchanged, 1 failed"
        );
        let one = Summary {
            sources: 1,
            unchanged: 1,
            ..Summary::default()
        };
        assert_eq!(
            one.to_string(),
            "1 source: 0 updated (0 new articles), 1 unchanged, 0 failed"
        );
    }
}
