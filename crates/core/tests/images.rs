//! The image worker answers every request, and only with real images.
//! Served from 127.0.0.1; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use gutenberg_core::images::{Format, Images, Loaded};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// `/icon.png` is a PNG, `/page` an HTML page, `/huge` claims 20 MB, and
/// anything else is a 404.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let png = std::fs::read(format!("{FIXTURES}/icons/archlinux.png")).unwrap();
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
            let head = String::from_utf8_lossy(&request).into_owned();
            let (status, length, body): (&str, usize, Vec<u8>) =
                match head.split_whitespace().nth(1).unwrap_or("/") {
                    "/icon.png" => ("200 OK", png.len(), png.clone()),
                    "/page" => {
                        let page = b"<!DOCTYPE html><html><body>Not here</body></html>".to_vec();
                        ("200 OK", page.len(), page)
                    }
                    "/huge" => ("200 OK", 20 * 1024 * 1024, Vec::new()),
                    _ => ("404 Not Found", 0, Vec::new()),
                };
            let mut response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
            )
            .into_bytes();
            response.extend_from_slice(&body);
            let _ = stream.write_all(&response);
        }
    });
    base
}

#[test]
fn every_request_is_answered_and_only_images_load() {
    let base = serve();
    let (tx, rx) = mpsc::channel();
    let images = Images::spawn(move |loaded| tx.send(loaded).unwrap()).unwrap();
    for path in ["/icon.png", "/page", "/huge", "/gone"] {
        images.request(format!("{base}{path}"));
    }

    let mut loaded = Vec::new();
    let mut failed = Vec::new();
    for _ in 0..4 {
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Loaded::Image { url, bytes, format } => loaded.push((url, bytes.len(), format)),
            Loaded::Failed { url } => failed.push(url),
        }
    }
    let png = std::fs::metadata(format!("{FIXTURES}/icons/archlinux.png")).unwrap();
    assert_eq!(
        loaded,
        [(format!("{base}/icon.png"), png.len() as usize, Format::Png)]
    );
    failed.sort();
    assert_eq!(
        failed,
        [
            format!("{base}/gone"),
            format!("{base}/huge"),
            format!("{base}/page")
        ]
    );
}
