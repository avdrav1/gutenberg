//! The release check asks GitHub at most once a day and keeps what it
//! heard. GitHub is a local server here; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use gutenberg_core::update::{check, latest, EVERY};
use gutenberg_sources::feed;

/// Answers every request with `body` and a 200, counting them; or a 503
/// once `down` is set.
fn serve(body: &'static str) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/releases/latest", listener.local_addr().unwrap());
    let (asked, down) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (counter, off) = (asked.clone(), down.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buf[..n]),
                }
            }
            counter.fetch_add(1, Ordering::SeqCst);
            let response = if off.load(Ordering::SeqCst) > 0 {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (url, asked, down)
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The fields of a real answer that matter, and one that doesn't.
const RELEASE: &str = r#"{"tag_name":"v0.25.0","name":"v0.25.0","draft":false,"assets":[]}"#;

#[test]
fn the_latest_release_is_its_tag_without_the_v() {
    let (url, ..) = serve(RELEASE);
    let client = feed::client().unwrap();
    assert_eq!(
        runtime().block_on(latest(&client, &url)),
        Ok("0.25.0".into())
    );
    let (url, ..) = serve(r#"{"tag_name":"nightly"}"#);
    assert!(runtime().block_on(latest(&client, &url)).is_err());
}

#[test]
fn github_is_asked_once_a_day_and_an_outage_keeps_the_last_answer() {
    let (url, asked, down) = serve(RELEASE);
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("latest-release");
    let client = feed::client().unwrap();
    let rt = runtime();
    let at = |now| rt.block_on(check(&client, &url, &cache, now));

    assert_eq!(at(1_000).as_deref(), Some("0.25.0"));
    assert_eq!(at(1_000 + EVERY - 1).as_deref(), Some("0.25.0"));
    assert_eq!(asked.load(Ordering::SeqCst), 1);

    // A day on, GitHub is asked again; down, the cached answer stands.
    down.store(1, Ordering::SeqCst);
    assert_eq!(at(1_000 + EVERY).as_deref(), Some("0.25.0"));
    assert_eq!(asked.load(Ordering::SeqCst), 2);

    // Nothing cached and GitHub down: no answer.
    std::fs::remove_file(&cache).unwrap();
    assert_eq!(at(9_999_999), None);
}
