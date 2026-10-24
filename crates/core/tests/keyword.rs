//! S5.2 acceptance: a keyword alert asks Google News and Bing News, and the
//! same story from both appears once; Google News links move to the
//! publisher's URL a few at a time. The providers are a local server on
//! 127.0.0.1 answering with the captured searches; nothing leaves the
//! machine. S5.3: each provider is asked the query as it reads it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gutenberg_core::keyword::{add, Keyword};
use gutenberg_core::sync::{refresh, Limits, RESOLVE_PER_REFRESH};
use gutenberg_sources::keyword::Providers;
use gutenberg_store::Store;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// The fixture article page's id, which the server swaps for the one asked.
const PAGE_ID: &str = "CBMimgFBVV95cUxPSHBRZ0prS1AxTTdwenpQc2twNHc3RkFsb3h4dW5mbDU2Z0NfLThqMFJBNllKT1ItZDgwVkJhZTFycHVPWGRpLVM2U3k3aFhhcXZzN1hZSDJnZzR2UmtWaktoTWEzLWVGd05yX2tocEozNnNjMTVJUnptYy00cXd3cks1bTJBNkJmVUlmYVJiWTUwMWROTWdvclJR";

#[derive(Default)]
struct Server {
    bing_down: AtomicBool,
    google_down: AtomicBool,
    resolves: AtomicUsize,
    /// The path of every search asked.
    searches: Mutex<Vec<String>>,
}

/// Google News and Bing News at one local address. Google's article links
/// point back at it; each resolves to `https://publisher.example/<id>`.
fn serve(server: Arc<Server>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let google = std::fs::read_to_string(format!("{FIXTURES}/feeds/google-news-cory-booker.xml"))
        .unwrap()
        .replace(
            "https://news.google.com/rss/articles/",
            &format!("{base}/rss/articles/"),
        );
    let bing = std::fs::read(format!("{FIXTURES}/feeds/bing-news-cory-booker.xml")).unwrap();
    let page =
        std::fs::read_to_string(format!("{FIXTURES}/pages/google-news-article.html")).unwrap();
    let (google, bing, page) = (Arc::new(google), Arc::new(bing), Arc::new(page));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (server, google, bing, page) =
                (server.clone(), google.clone(), bing.clone(), page.clone());
            std::thread::spawn(move || {
                let mut stream = stream.unwrap();
                let (path, body) = read_request(&mut stream);
                if path.contains("/search?") {
                    server.searches.lock().unwrap().push(path.clone());
                }
                let reply = if path.starts_with("/rss/search") {
                    (!server.google_down.load(Ordering::SeqCst)).then(|| google.as_bytes().to_vec())
                } else if path.starts_with("/news/search") {
                    (!server.bing_down.load(Ordering::SeqCst)).then(|| bing.to_vec())
                } else if let Some(rest) = path.strip_prefix("/rss/articles/") {
                    let id = rest.split('?').next().unwrap();
                    Some(page.replace(PAGE_ID, id).into_bytes())
                } else if path.starts_with("/_/DotsSplashUi/data/batchexecute") {
                    server.resolves.fetch_add(1, Ordering::SeqCst);
                    let form: String = url::form_urlencoded::parse(&body)
                        .find(|(k, _)| k == "f.req")
                        .unwrap()
                        .1
                        .into_owned();
                    let outer: serde_json::Value = serde_json::from_str(&form).unwrap();
                    let inner: serde_json::Value =
                        serde_json::from_str(outer[0][0][1].as_str().unwrap()).unwrap();
                    let id = inner[2].as_str().unwrap();
                    let res = serde_json::json!([
                        "garturlres",
                        format!("https://publisher.example/{}", &id[..24]),
                        1
                    ]);
                    let rows = serde_json::json!([["wrb.fr", "Fbv4je", res.to_string(), null]]);
                    Some(format!(")]}}'\n\n{rows}").into_bytes())
                } else {
                    None
                };
                let response = match reply {
                    Some(body) => {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&body);
                        r
                    }
                    None => b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_vec(),
                };
                let _ = stream.write_all(&response);
            });
        }
    });
    base
}

/// The request's path and body.
fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut request = Vec::new();
    let mut buf = [0; 4096];
    let head_end = loop {
        if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return (String::new(), Vec::new()),
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).into_owned();
    let length: usize = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().to_owned())
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while request.len() < head_end + length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    (path, request[head_end..].to_vec())
}

fn limits(base: &str) -> Limits {
    Limits {
        per_host: 8,
        spacing: Duration::ZERO,
        providers: Providers {
            google_news: base.to_owned(),
            bing_news: base.to_owned(),
        },
        ..Limits::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// `(on Google, on a publisher)` among the alert's articles.
fn links(store: &Store, source_id: i64, base: &str) -> (usize, usize) {
    let rows = store.list_articles(Some(source_id), 1000).unwrap();
    let google = rows
        .iter()
        .filter(|r| r.url.as_deref().is_some_and(|u| u.starts_with(base)))
        .count();
    (google, rows.len() - google)
}

#[test]
fn the_same_story_from_both_providers_appears_once() {
    let server = Arc::new(Server::default());
    let base = serve(server.clone());
    let mut store = Store::open_in_memory().unwrap();
    let booker = Keyword::new("\"Cory Booker\"").unwrap();
    let alert = add(&store, &booker, "Cory Booker", None)
        .unwrap()
        .unwrap()
        .id;
    let rt = runtime();

    let first = rt
        .block_on(refresh(&mut store, 1_000, &limits(&base), None))
        .unwrap();
    // 10 from Bing and 100 from Google, less the 4 stories both carry and
    // one Google carries twice.
    assert_eq!((first.updated, first.new_articles), (1, 105), "{first}");
    assert!(first.failed.is_empty(), "{first}");
    assert_eq!(server.resolves.load(Ordering::SeqCst), RESOLVE_PER_REFRESH);
    // Bing's 10 name their publisher; so do the 20 Google links resolved.
    assert_eq!(links(&store, alert, &base), (75, 30));
    let rows = store.list_articles(Some(alert), 1000).unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r.title == "Could Cory Booker lose his NJ Senate seat? What forecasts show")
            .count(),
        1
    );

    // The same results again add nothing, and resolve 20 more old links.
    let second = rt
        .block_on(refresh(&mut store, 100_000, &limits(&base), None))
        .unwrap();
    assert_eq!(second.new_articles, 0, "{second}");
    assert_eq!(links(&store, alert, &base), (55, 50));
}

#[test]
fn one_provider_down_is_not_the_alert_failing() {
    let server = Arc::new(Server::default());
    server.bing_down.store(true, Ordering::SeqCst);
    let base = serve(server.clone());
    let mut store = Store::open_in_memory().unwrap();
    let booker = Keyword::new("\"Cory Booker\"").unwrap();
    let alert = add(&store, &booker, "Cory Booker", None)
        .unwrap()
        .unwrap()
        .id;
    let rt = runtime();

    let summary = rt
        .block_on(refresh(&mut store, 1_000, &limits(&base), None))
        .unwrap();
    // Google alone: its 100, less the one it carries twice.
    assert_eq!(summary.new_articles, 99, "{summary}");
    assert!(store.fetch_state(alert).unwrap().error.is_none());

    // Both down is a failure, and the alert backs off.
    server.google_down.store(true, Ordering::SeqCst);
    let summary = rt
        .block_on(refresh(&mut store, 100_000, &limits(&base), None))
        .unwrap();
    assert_eq!(summary.failed.len(), 1, "{summary}");
    let state = store.fetch_state(alert).unwrap();
    assert_eq!(state.error.as_deref(), Some("HTTP 503 Service Unavailable"));
    assert_eq!(state.failures, 1);
}

#[test]
fn each_provider_is_asked_what_it_reads() {
    let server = Arc::new(Server::default());
    let base = serve(server.clone());
    let mut store = Store::open_in_memory().unwrap();
    let query = "\"Cory Booker\" -Senate region:GB";
    let alert = add(&store, &Keyword::new(query).unwrap(), query, None)
        .unwrap()
        .unwrap()
        .id;
    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &limits(&base), None))
        .unwrap();
    let mut searches = server.searches.lock().unwrap().clone();
    searches.sort();
    assert_eq!(
        searches,
        [
            "/news/search?q=%22Cory+Booker%22&format=rss&mkt=en-GB",
            "/rss/search?q=%22Cory+Booker%22+-Senate&hl=en-GB&gl=GB&ceid=GB%3Aen",
        ]
    );
    // Neither fixture was searched with the exclusion, so it falls to the
    // results: 20 of the 105 mention the Senate in headline or summary.
    assert_eq!(summary.new_articles, 85, "{summary}");
    let rows = store.list_articles(Some(alert), 1000).unwrap();
    assert!(rows
        .iter()
        .all(|r| !r.title.to_lowercase().contains("senate")));
}

#[test]
fn bing_sits_out_a_query_it_cannot_ask() {
    let server = Arc::new(Server::default());
    let base = serve(server.clone());
    let mut store = Store::open_in_memory().unwrap();
    let query = "\"Cory Booker\" OR \"Andy Kim\"";
    let alert = add(&store, &Keyword::new(query).unwrap(), query, None)
        .unwrap()
        .unwrap()
        .id;
    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &limits(&base), None))
        .unwrap();
    let searches = server.searches.lock().unwrap().clone();
    assert_eq!(searches.len(), 1, "{searches:?}");
    assert!(searches[0].starts_with("/rss/search?q=%22Cory+Booker%22+OR+%22Andy+Kim%22&"));
    assert_eq!(summary.new_articles, 99, "{summary}");
    assert!(store.fetch_state(alert).unwrap().error.is_none());
}
