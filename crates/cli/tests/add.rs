//! `gutenberg --add` end to end. The feed is served from a fixture by a
//! one-shot HTTP server on 127.0.0.1, so nothing leaves the machine.
//! The store goes to a temp dir: through `XDG_DATA_HOME` on Linux, and on
//! macOS through `HOME`, which `directories` reads for `~/Library`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/feeds");

/// Serves one response, then stops. Returns the URL to fetch.
fn serve_once(status: &str, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/feed", listener.local_addr().unwrap());
    let status = status.to_owned();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
        }
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    url
}

/// Serves fixed responses by path until the test ends; anything else is
/// a 404. Returns the base URL, with no trailing slash.
fn serve(routes: Vec<(&'static str, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
            }
            let line = String::from_utf8_lossy(&request);
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => ("200 OK", body.clone()),
                None => ("404 Not Found", b"not here".to_vec()),
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    base
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/../{path}")).unwrap()
}

/// Where the child finds its store, and where that store lands.
const DATA_HOME: &str = if cfg!(target_os = "macos") {
    "HOME"
} else {
    "XDG_DATA_HOME"
};
const STORE: &str = if cfg!(target_os = "macos") {
    "Library/Application Support/gutenberg/gutenberg.db"
} else {
    "gutenberg/gutenberg.db"
};

fn gutenberg(data_home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutenberg"))
        .args(args)
        .env(DATA_HOME, data_home)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn adds_a_feed_and_its_articles_once() {
    let data = tempfile::tempdir().unwrap();
    let body = std::fs::read(format!("{FIXTURES}/archlinux-news.xml")).unwrap();
    let url = serve_once("200 OK", body);

    let out = gutenberg(data.path(), &["--add", &url]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout).trim(),
        "Added Arch Linux: Recent news updates: 10 articles"
    );

    let store = gutenberg_store::Store::open(&data.path().join(STORE)).unwrap();
    assert_eq!(
        store.sources().unwrap()[0].url.as_deref(),
        Some(url.as_str())
    );
    assert_eq!(store.list_articles(None, 100).unwrap().len(), 10);

    // A second add stops before the network: nothing is listening now.
    let again = gutenberg(data.path(), &["--add", &url]);
    assert!(!again.status.success());
    assert_eq!(
        text(&again.stderr).trim(),
        format!("gutenberg: {url}: already added")
    );
}

#[test]
fn bad_xml_is_one_line_of_error() {
    let data = tempfile::tempdir().unwrap();
    let body = std::fs::read(format!("{FIXTURES}/bad/truncated.xml")).unwrap();
    let url = serve_once("200 OK", body);

    let out = gutenberg(data.path(), &["--add", &url]);
    assert!(!out.status.success());
    let stderr = text(&out.stderr);
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with(&format!("gutenberg: {url}: not a feed (")),
        "{stderr}"
    );
}

#[test]
fn an_http_error_is_one_line() {
    let data = tempfile::tempdir().unwrap();
    let url = serve_once("404 Not Found", b"gone".to_vec());

    let out = gutenberg(data.path(), &["--add", &url]);
    assert!(!out.status.success());
    assert_eq!(
        text(&out.stderr).trim(),
        format!("gutenberg: {url}: HTTP 404 Not Found")
    );
}

#[test]
fn add_without_a_url_prints_usage() {
    let data = tempfile::tempdir().unwrap();
    let out = gutenberg(data.path(), &["--add"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).starts_with("usage: gutenberg"));
}

#[test]
fn a_site_address_finds_the_feed_its_page_links_to() {
    let data = tempfile::tempdir().unwrap();
    // Arch's home page links its news feed as /feeds/news/.
    let base = serve(vec![
        ("/", fixture("pages/archlinux.html")),
        ("/feeds/news/", fixture("feeds/archlinux-news.xml")),
    ]);
    let site = format!("{base}/");
    let feed = format!("{base}/feeds/news/");

    let out = gutenberg(data.path(), &["--add", &site]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!("Found {feed}\nAdded Arch Linux: Recent news updates: 10 articles\n")
    );
    let store = gutenberg_store::Store::open(&data.path().join(STORE)).unwrap();
    assert_eq!(
        store.sources().unwrap()[0].url.as_deref(),
        Some(feed.as_str())
    );

    // Pasting the site again finds the same feed, already there.
    let again = gutenberg(data.path(), &["--add", &site]);
    assert!(!again.status.success());
    assert_eq!(
        text(&again.stderr).trim(),
        format!("gutenberg: {site}: already added as {feed}")
    );
}

#[test]
fn a_page_without_links_falls_back_to_common_paths() {
    let data = tempfile::tempdir().unwrap();
    // No <link>, no /feed: /rss is next.
    let base = serve(vec![
        ("/", fixture("pages/example.html")),
        ("/rss", fixture("feeds/lwn-headlines.xml")),
    ]);

    let out = gutenberg(data.path(), &["--add", &format!("{base}/")]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        format!("Found {base}/rss\nAdded LWN.net: 10 articles\n")
    );
}

#[test]
fn a_site_with_no_feed_says_so_in_one_line() {
    let data = tempfile::tempdir().unwrap();
    let base = serve(vec![("/", fixture("pages/example.html"))]);
    let site = format!("{base}/");

    let out = gutenberg(data.path(), &["--add", &site]);
    assert!(!out.status.success());
    assert_eq!(
        text(&out.stderr),
        format!(
            "gutenberg: {site}: no feed found (no <link rel=\"alternate\"> on the page; \
             /feed, /rss, /atom.xml didn't parse)\n"
        )
    );
}
