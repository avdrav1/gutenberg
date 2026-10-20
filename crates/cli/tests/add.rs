//! `gutenberg --add` end to end. The feed is served from a fixture by a
//! one-shot HTTP server on 127.0.0.1, so nothing leaves the machine.
//! Linux only: `XDG_DATA_HOME` points the store at a temp dir there.
#![cfg(target_os = "linux")]

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

fn gutenberg(data_home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutenberg"))
        .args(args)
        .env("XDG_DATA_HOME", data_home)
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

    let store = gutenberg_store::Store::open(&data.path().join("gutenberg/gutenberg.db")).unwrap();
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
