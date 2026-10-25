//! The web worker answers every request: images only with real images,
//! pages with their article or one line on why not. Served from
//! 127.0.0.1; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use gutenberg_core::web::{Answer, Format, Keep, Web};
use gutenberg_store::{NewArticle, NewSource, SourceKind, Store};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures");

/// `/icon.png` is a PNG, `/article` a captured Phoronix page, `/page` an
/// empty HTML page, `/huge` claims 20 MB, and anything else is a 404.
fn serve() -> String {
    serve_counting().0
}

/// [`serve`], counting the requests it answers.
fn serve_counting() -> (String, Arc<AtomicUsize>) {
    let served = Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
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
            counter.fetch_add(1, Ordering::SeqCst);
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
    (base, served)
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

#[test]
fn what_was_fetched_comes_from_the_cache_next_time_and_pages_are_searched() {
    // S7.4: the second image and page come off disk, and the page's words
    // reach the article's search index.
    let (base, served) = serve_counting();
    let tmp = tempfile::tempdir().unwrap();
    let store_path = tmp.path().join("gutenberg.db");
    let mut store = Store::open(&store_path).unwrap();
    let source = store
        .add_source(&NewSource {
            folder_id: None,
            kind: SourceKind::Feed,
            title: "Phoronix".into(),
            url: Some(format!("{base}/feed")),
            config: "{}".into(),
        })
        .unwrap();
    store
        .insert_articles(&[NewArticle {
            source_id: source.id,
            guid: "1".into(),
            url: Some(format!("{base}/article")),
            title: "Linux news".into(),
            author: None,
            summary_html: None,
            published_at: Some(1),
            fetched_at: 1,
        }])
        .unwrap();
    assert!(store.search("workqueue", 10).unwrap().is_empty());

    let keep = || Keep {
        cache: Some((tmp.path().join("cache"), 1 << 20)),
        store: Some(store_path.clone()),
    };
    let fetch_both = |keep: Keep| {
        let (tx, rx) = mpsc::channel();
        let web = Web::spawn_keeping(keep, move |answer| tx.send(answer).unwrap()).unwrap();
        web.image(format!("{base}/icon.png"));
        web.page(format!("{base}/article"));
        let mut all: Vec<Answer> = (0..2)
            .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect();
        all.sort_by_key(|a| matches!(a, Answer::Page { .. }));
        assert!(
            matches!(
                &all[0],
                Answer::Image {
                    format: Format::Png,
                    ..
                }
            ),
            "{:?}",
            all[0]
        );
        assert!(
            matches!(&all[1], Answer::Page { html, .. } if html.contains("merged today for Linux 7.4")),
            "{:?}",
            all[1]
        );
    };
    fetch_both(keep());
    assert_eq!(served.load(Ordering::SeqCst), 2);
    assert_eq!(store.search("workqueue", 10).unwrap().len(), 1);

    // A new worker, as at the next launch: nothing is fetched.
    fetch_both(keep());
    assert_eq!(served.load(Ordering::SeqCst), 2);
}
