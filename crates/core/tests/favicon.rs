//! S2.6 acceptance: a feed's site icon is fetched once and cached on disk;
//! later refreshes send no request for it. Sites are fixtures served from
//! 127.0.0.1; nothing leaves the machine.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use gutenberg_core::favicon::Icons;
use gutenberg_core::sync::{refresh, Limits};
use gutenberg_store::{NewSource, SourceKind, Store};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");
const ETAG: &str = "\"v1\"";

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{path}")).unwrap()
}

/// What a path answers. Feeds carry `ETAG` and answer it with a 304.
#[derive(Clone)]
enum Route {
    Feed(Vec<u8>),
    Body(Vec<u8>),
    Status(&'static str),
}

/// Requests seen: the path and its `If-None-Match`, in arrival order.
type Log = Arc<Mutex<Vec<(String, Option<String>)>>>;

struct Site {
    base: String,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    log: Log,
}

impl Site {
    /// A server at `base` whose `/feed.xml` names `base/` as its site.
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let log = Log::default();
        let (r, l) = (routes.clone(), log.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (r, l) = (r.clone(), l.clone());
                std::thread::spawn(move || handle(stream.unwrap(), &r, &l));
            }
        });
        let feed = String::from_utf8(fixture("feeds/lwn-headlines.xml"))
            .unwrap()
            .replace(
                "<link>https://lwn.net</link>",
                &format!("<link>{base}/</link>"),
            );
        let site = Self { base, routes, log };
        site.route("/feed.xml", Route::Feed(feed.into_bytes()));
        site
    }

    fn route(&self, path: &str, route: Route) {
        self.routes.lock().unwrap().insert(path.to_owned(), route);
    }

    /// Paths asked for since the last call, with their `If-None-Match`.
    fn take_log(&self) -> Vec<(String, Option<String>)> {
        std::mem::take(&mut *self.log.lock().unwrap())
    }

    fn feed_url(&self) -> String {
        format!("{}/feed.xml", self.base)
    }
}

fn handle(mut stream: TcpStream, routes: &Mutex<HashMap<String, Route>>, log: &Log) {
    let mut request = Vec::new();
    let mut buf = [0; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&request).into_owned();
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let if_none_match = head.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.eq_ignore_ascii_case("if-none-match")
            .then(|| value.trim().to_owned())
    });
    log.lock()
        .unwrap()
        .push((path.clone(), if_none_match.clone()));

    let route = routes.lock().unwrap().get(&path).cloned();
    let (status, etag, body) = match route {
        Some(Route::Feed(_)) if if_none_match.as_deref() == Some(ETAG) => {
            ("304 Not Modified", None, Vec::new())
        }
        Some(Route::Feed(body)) => ("200 OK", Some(ETAG), body),
        Some(Route::Body(body)) => ("200 OK", None, body),
        Some(Route::Status(status)) => (status, None, Vec::new()),
        None => ("404 Not Found", None, Vec::new()),
    };
    let etag = etag.map_or(String::new(), |e| format!("ETag: {e}\r\n"));
    let mut response = format!(
        "HTTP/1.1 {status}\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    let _ = stream.write_all(&response);
}

fn store_with(feed_url: &str) -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: feed_url.to_owned(),
            url: Some(feed_url.to_owned()),
            config: "{}".to_owned(),
        })
        .unwrap();
    store
}

fn run(store: &mut Store, now: i64, icons: Option<&Icons>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(refresh(store, now, &Limits::default(), icons))
        .unwrap();
}

fn paths(log: &[(String, Option<String>)]) -> Vec<&str> {
    log.iter().map(|(path, _)| path.as_str()).collect()
}

const ARCH_ICON: &str = "/static/archlinux_common_style/favicon.png";

#[test]
fn the_icon_is_fetched_once_and_never_asked_for_again() {
    let site = Site::new();
    site.route("/", Route::Body(fixture("pages/archlinux.html")));
    site.route(ARCH_ICON, Route::Body(fixture("icons/archlinux.png")));
    let dir = tempfile::tempdir().unwrap();
    let icons = Icons::new(dir.path());
    let mut store = store_with(&site.feed_url());

    run(&mut store, 1_000, Some(&icons));
    assert_eq!(paths(&site.take_log()), ["/feed.xml", "/", ARCH_ICON]);
    let found = icons.found();
    let path = &found[&Icons::key(&site.feed_url())];
    assert_eq!(path.extension().unwrap(), "png");
    assert_eq!(std::fs::read(path).unwrap(), fixture("icons/archlinux.png"));

    // The next refresh is the feed's conditional GET and nothing else.
    run(&mut store, 2_000, Some(&icons));
    assert_eq!(
        site.take_log(),
        [("/feed.xml".to_owned(), Some(ETAG.to_owned()))]
    );
}

#[test]
fn a_site_without_an_icon_is_asked_once() {
    let site = Site::new();
    // example.com names only a data: icon, which can't be fetched.
    site.route("/", Route::Body(fixture("pages/example.html")));
    let dir = tempfile::tempdir().unwrap();
    let icons = Icons::new(dir.path());
    let mut store = store_with(&site.feed_url());

    run(&mut store, 1_000, Some(&icons));
    // No <link rel="icon">, and /favicon.ico is a 404.
    assert_eq!(paths(&site.take_log()), ["/feed.xml", "/", "/favicon.ico"]);
    assert!(icons.found().is_empty());

    run(&mut store, 2_000, Some(&icons));
    assert_eq!(paths(&site.take_log()), ["/feed.xml"]);
}

#[test]
fn a_feed_fetched_before_icons_existed_is_fetched_in_full_once() {
    let site = Site::new();
    site.route("/", Route::Body(fixture("pages/archlinux.html")));
    site.route(ARCH_ICON, Route::Body(fixture("icons/archlinux.png")));
    let dir = tempfile::tempdir().unwrap();
    let icons = Icons::new(dir.path());
    let mut store = store_with(&site.feed_url());
    run(&mut store, 1_000, None);
    site.take_log();

    // A 304 carries no site link, so the feed comes whole this one time.
    run(&mut store, 2_000, Some(&icons));
    let log = site.take_log();
    assert_eq!(log[0], ("/feed.xml".to_owned(), None));
    assert_eq!(paths(&log), ["/feed.xml", "/", ARCH_ICON]);

    run(&mut store, 3_000, Some(&icons));
    assert_eq!(
        site.take_log(),
        [("/feed.xml".to_owned(), Some(ETAG.to_owned()))]
    );
}

#[test]
fn a_site_that_is_down_is_asked_again_next_refresh() {
    let site = Site::new();
    site.route("/", Route::Status("503 Service Unavailable"));
    let dir = tempfile::tempdir().unwrap();
    let icons = Icons::new(dir.path());
    let mut store = store_with(&site.feed_url());

    run(&mut store, 1_000, Some(&icons));
    assert_eq!(paths(&site.take_log()), ["/feed.xml", "/"]);
    assert!(icons.settled().is_empty());

    site.route("/", Route::Body(fixture("pages/archlinux.html")));
    site.route(ARCH_ICON, Route::Body(fixture("icons/archlinux.png")));
    run(&mut store, 2_000, Some(&icons));
    assert_eq!(paths(&site.take_log()), ["/feed.xml", "/", ARCH_ICON]);
    assert_eq!(icons.found().len(), 1);
}
