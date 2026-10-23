//! The web worker answers every request: images only with real images,
//! pages with their article or one line on why not. Served from
//! 127.0.0.1; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use gutenberg_core::web::{Answer, Format, Web};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// `/icon.png` is a PNG, `/article` a captured Phoronix page, `/page` an
/// empty HTML page, `/huge` claims 20 MB, and anything else is a 404.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let png = std::fs::read(format!("{FIXTURES}/icons/archlinux.png")).unwrap();
    let article = std::fs::read(format!("{FIXTURES}/pages/reader/phoronix.html")).unwrap();
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
                    "/article" => ("200 OK", article.len(), article.clone()),
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

fn answers(web_requests: impl FnOnce(&Web), count: usize) -> Vec<Answer> {
    let (tx, rx) = mpsc::channel();
    let web = Web::spawn(move |answer| tx.send(answer).unwrap()).unwrap();
    web_requests(&web);
    (0..count)
        .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
        .collect()
}

#[test]
fn every_image_request_is_answered_and_only_images_load() {
    let base = serve();
    let mut loaded = Vec::new();
    let mut failed = Vec::new();
    let all = answers(
        |web| {
            for path in ["/icon.png", "/page", "/huge", "/gone"] {
                web.image(format!("{base}{path}"));
            }
        },
        4,
    );
    for answer in all {
        match answer {
            Answer::Image { url, bytes, format } => loaded.push((url, bytes.len(), format)),
            Answer::ImageFailed { url } => failed.push(url),
            other => panic!("{other:?}"),
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

#[test]
fn a_page_comes_back_as_its_article_or_a_reason() {
    let base = serve();
    let mut all = answers(
        |web| {
            web.page(format!("{base}/article"));
            web.page(format!("{base}/gone"));
        },
        2,
    );
    all.sort_by_key(|a| matches!(a, Answer::PageFailed { .. }));
    match &all[0] {
        Answer::Page { url, html } => {
            assert_eq!(url, &format!("{base}/article"));
            assert!(html.contains("merged today for Linux 7.4"), "{html}");
        }
        other => panic!("{other:?}"),
    }
    match &all[1] {
        Answer::PageFailed { url, error } => {
            assert_eq!(url, &format!("{base}/gone"));
            assert_eq!(error, "HTTP 404 Not Found");
        }
        other => panic!("{other:?}"),
    }
}
