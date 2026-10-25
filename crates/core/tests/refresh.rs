//! S2.3 acceptance: a second refresh of an unchanged feed sends
//! `If-None-Match` and gets 304; 100 feeds finish in under 10 s. Feeds are
//! fixtures served by local HTTP servers on 127.0.0.1; nothing leaves the
//! machine. S8.4: rules meet what a refresh brings.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gutenberg_core::rules::{Action, Rule, Rules};
use gutenberg_core::sync::{refresh, Limits};
use gutenberg_store::{NewSource, SourceKind, Store};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/feeds");

/// What the server saw and how busy it got.
#[derive(Default)]
struct Log {
    /// `If-None-Match` of each request, in arrival order.
    if_none_match: Mutex<Vec<Option<String>>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

#[derive(Clone)]
enum Reply {
    /// The body with this ETag; 304 when the request already has it.
    Feed { body: Arc<Vec<u8>>, etag: String },
    /// A status with an optional `Retry-After`.
    Status(&'static str, Option<u64>),
}

/// A server that answers every request with `reply` after `delay`, one
/// thread per connection so requests can overlap. Returns its base URL.
fn serve(reply: Arc<Mutex<Reply>>, delay: Duration, log: Arc<Log>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (reply, log) = (reply.clone(), log.clone());
            std::thread::spawn(move || handle(stream.unwrap(), &reply, delay, &log));
        }
    });
    base
}

fn handle(mut stream: TcpStream, reply: &Mutex<Reply>, delay: Duration, log: &Log) {
    let mut request = Vec::new();
    let mut buf = [0; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let now = log.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    log.max_in_flight.fetch_max(now, Ordering::SeqCst);
    let head = String::from_utf8_lossy(&request).to_ascii_lowercase();
    let if_none_match = head
        .lines()
        .find_map(|l| l.strip_prefix("if-none-match:"))
        .map(|v| v.trim().to_owned());
    log.if_none_match
        .lock()
        .unwrap()
        .push(if_none_match.clone());
    std::thread::sleep(delay);

    let reply = reply.lock().unwrap().clone();
    let response: Vec<u8> = match reply {
        Reply::Feed { body, etag } => {
            if if_none_match.as_deref() == Some(etag.to_ascii_lowercase().as_str()) {
                b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec()
            } else {
                let mut r = format!(
                    "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                r.extend_from_slice(&body);
                r
            }
        }
        Reply::Status(status, retry_after) => {
            let retry = retry_after.map_or(String::new(), |s| format!("Retry-After: {s}\r\n"));
            format!("HTTP/1.1 {status}\r\n{retry}Content-Length: 0\r\nConnection: close\r\n\r\n")
                .into_bytes()
        }
    };
    log.in_flight.fetch_sub(1, Ordering::SeqCst);
    let _ = stream.write_all(&response);
}

fn feed_reply(fixture: &str, etag: &str) -> Arc<Mutex<Reply>> {
    let body = std::fs::read(format!("{FIXTURES}/{fixture}")).unwrap();
    Arc::new(Mutex::new(Reply::Feed {
        body: Arc::new(body),
        etag: etag.to_owned(),
    }))
}

fn add_feed(store: &Store, url: &str) -> i64 {
    store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: url.to_owned(),
            url: Some(url.to_owned()),
            config: "{}".to_owned(),
        })
        .unwrap()
        .id
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn a_second_refresh_of_an_unchanged_feed_gets_304() {
    let log = Arc::new(Log::default());
    let base = serve(
        feed_reply("lwn-headlines.xml", "\"lwn-1\""),
        Duration::ZERO,
        log.clone(),
    );
    let mut store = Store::open_in_memory().unwrap();
    let id = add_feed(&store, &format!("{base}/headlines/rss"));
    let rt = runtime();

    let first = rt
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    assert_eq!((first.updated, first.new_articles), (1, 10), "{first}");
    assert_eq!(
        store.fetch_state(id).unwrap().validators.etag.as_deref(),
        Some("\"lwn-1\"")
    );

    let second = rt
        .block_on(refresh(&mut store, 2_000, &Limits::default(), None))
        .unwrap();
    assert_eq!((second.unchanged, second.new_articles), (1, 0), "{second}");
    assert_eq!(
        *log.if_none_match.lock().unwrap(),
        [None, Some("\"lwn-1\"".to_owned())]
    );
    assert_eq!(store.list_articles(None, 100).unwrap().len(), 10);
    assert_eq!(store.fetch_state(id).unwrap().checked_at, Some(2_000));
}

#[test]
fn a_hundred_feeds_finish_in_under_ten_seconds() {
    // 10 hosts (servers on different ports), 10 feeds each, 100 ms a reply.
    let mut store = Store::open_in_memory().unwrap();
    let logs: Vec<Arc<Log>> = (0..10).map(|_| Arc::new(Log::default())).collect();
    for log in &logs {
        let base = serve(
            feed_reply("archlinux-news.xml", "\"a\""),
            Duration::from_millis(100),
            log.clone(),
        );
        for feed in 0..10 {
            add_feed(&store, &format!("{base}/feed/{feed}"));
        }
    }

    let start = Instant::now();
    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    let took = start.elapsed();

    assert_eq!(summary.sources, 100);
    assert!(summary.failed.is_empty(), "{:?}", summary.failed);
    assert_eq!(summary.updated, 100);
    assert!(took < Duration::from_secs(10), "took {took:?}");
    for log in &logs {
        assert!(log.max_in_flight.load(Ordering::SeqCst) <= 2);
    }
    println!("100 feeds: {took:?}");
}

#[test]
fn retry_after_backs_a_feed_off_until_then() {
    let reply = Arc::new(Mutex::new(Reply::Status(
        "503 Service Unavailable",
        Some(120),
    )));
    let base = serve(reply.clone(), Duration::ZERO, Arc::new(Log::default()));
    let mut store = Store::open_in_memory().unwrap();
    let id = add_feed(&store, &format!("{base}/feed"));
    let rt = runtime();

    let summary = rt
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    assert_eq!(summary.failed.len(), 1);
    let state = store.fetch_state(id).unwrap();
    assert_eq!(state.retry_at, Some(1_120));
    assert_eq!(state.error.as_deref(), Some("HTTP 503 Service Unavailable"));

    // Not due before then: nothing is fetched.
    let skipped = rt
        .block_on(refresh(&mut store, 1_100, &Limits::default(), None))
        .unwrap();
    assert_eq!(skipped.sources, 0);

    // A 500 with no Retry-After doubles from 5 minutes: second failure, 10.
    *reply.lock().unwrap() = Reply::Status("500 Internal Server Error", None);
    rt.block_on(refresh(&mut store, 1_200, &Limits::default(), None))
        .unwrap();
    let state = store.fetch_state(id).unwrap();
    assert_eq!((state.failures, state.retry_at), (2, Some(1_200 + 600)));

    // Back up: the failure is forgotten.
    *reply.lock().unwrap() = feed_reply("lwn-headlines.xml", "\"x\"")
        .lock()
        .unwrap()
        .clone();
    let ok = rt
        .block_on(refresh(&mut store, 1_800, &Limits::default(), None))
        .unwrap();
    assert_eq!(ok.new_articles, 10);
    let state = store.fetch_state(id).unwrap();
    assert_eq!(
        (state.failures, state.retry_at, state.error),
        (0, None, None)
    );
}

#[test]
fn a_feed_that_stops_parsing_records_one_line_and_backs_off() {
    let body = std::fs::read(format!("{FIXTURES}/bad/truncated.xml")).unwrap();
    let reply = Arc::new(Mutex::new(Reply::Feed {
        body: Arc::new(body),
        etag: "\"t\"".to_owned(),
    }));
    let base = serve(reply, Duration::ZERO, Arc::new(Log::default()));
    let mut store = Store::open_in_memory().unwrap();
    let id = add_feed(&store, &format!("{base}/feed"));

    runtime()
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    let state = store.fetch_state(id).unwrap();
    let error = state.error.unwrap();
    assert!(error.starts_with("not a feed ("), "{error}");
    assert!(!error.contains('\n'));
    assert_eq!(state.retry_at, Some(1_300));
}

#[test]
fn a_keyword_alert_without_a_query_fails_alone_and_backs_off() {
    let log = Arc::new(Log::default());
    let base = serve(
        feed_reply("lwn-headlines.xml", "\"l\""),
        Duration::ZERO,
        log.clone(),
    );
    let mut store = Store::open_in_memory().unwrap();
    let feed = add_feed(&store, &format!("{base}/feed"));
    let alert = store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Keyword,
            title: "Broken".to_owned(),
            url: None,
            config: "{}".to_owned(),
        })
        .unwrap()
        .id;

    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    // Settings that can't be read never reach the network; the feed still
    // refreshes.
    assert_eq!((summary.sources, summary.updated), (2, 1), "{summary}");
    assert_eq!(log.if_none_match.lock().unwrap().len(), 1);
    let state = store.fetch_state(alert).unwrap();
    let error = state.error.unwrap();
    assert!(error.starts_with("keyword alert settings: "), "{error}");
    assert_eq!((state.failures, state.retry_at), (1, Some(1_300)));
    assert!(store.fetch_state(feed).unwrap().error.is_none());
}

#[test]
fn a_refresh_asked_for_runs_even_with_background_refresh_off() {
    // S7.3: `gutenberg --refresh` with a window open goes to its refresh
    // thread, which answers with the summary.
    let log = Arc::new(Log::default());
    let base = serve(
        feed_reply("lwn-headlines.xml", "\"lwn-1\""),
        Duration::ZERO,
        log,
    );
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("gutenberg.db");
    add_feed(
        &Store::open(&path).unwrap(),
        &format!("{base}/headlines/rss"),
    );
    let mut config = gutenberg_core::config::Config::default();
    config.refresh.minutes = 0;
    let refresher = gutenberg_core::sync::spawn(path.clone(), None, &config);

    let summary = refresher.now().unwrap();
    assert_eq!(
        (summary.updated, summary.new_articles),
        (1, 10),
        "{summary}"
    );
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .list_articles(None, 100)
            .unwrap()
            .len(),
        10
    );
    // Asked again: answered again, with nothing new.
    assert_eq!(refresher.now().unwrap().new_articles, 0);
}

#[test]
fn rules_star_read_and_mute_what_a_refresh_brings() {
    // S8.4: of LWN's ten headlines, the two security update lists are
    // muted, the two on Rust starred and the kernel release marked read.
    let base = serve(
        feed_reply("lwn-headlines.xml", "\"lwn-1\""),
        Duration::ZERO,
        Arc::new(Log::default()),
    );
    let mut store = Store::open_in_memory().unwrap();
    let url = format!("{base}/headlines/rss");
    let id = add_feed(&store, &url);
    let rule = |words: &str, within: Option<&str>, action| Rule {
        words: words.into(),
        within: within.map(Into::into),
        action,
    };
    let limits = Limits {
        rules: Rules::new(&[
            rule("Security updates", None, Action::Mute),
            rule("rust", Some(&url), Action::Star),
            rule("7.3 kernel has been released", None, Action::Read),
            rule("rust", Some("Another feed"), Action::Mute),
        ]),
        ..Limits::default()
    };
    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &limits, None))
        .unwrap();
    assert_eq!((summary.new_articles, summary.muted), (8, 2));
    assert!(summary.to_string().ends_with(", 2 muted"), "{summary}");

    let articles = store.list_articles(Some(id), 100).unwrap();
    assert_eq!(articles.len(), 8);
    assert!(!articles
        .iter()
        .any(|a| a.title.starts_with("Security updates")));
    let mut starred: Vec<_> = articles
        .iter()
        .filter(|a| a.starred)
        .map(|a| a.title.as_str())
        .collect();
    starred.sort();
    assert_eq!(
        starred,
        [
            "[$] Cross-crate forward declarations in Rust",
            "[$] Safe self-referential data structures in kernel Rust",
        ]
    );
    let read: Vec<_> = articles
        .iter()
        .filter(|a| a.read)
        .map(|a| &a.title)
        .collect();
    assert_eq!(read, ["The 7.3 kernel has been released"]);
}
