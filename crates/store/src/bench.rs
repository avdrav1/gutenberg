//! For the S3.6 search budget: a large corpus with a known number of
//! matches, shared by `tests/search.rs` and `benches/search.rs`.

use crate::{NewArticle, NewSource, SourceKind, Store};

/// Articles in [`search_corpus`] that mention Harrisburg.
pub const HARRISBURG: usize = 37;

/// `n` articles (at least 100k for all [`HARRISBURG`] mentions) over 50
/// new feeds. Every 2,700th mentions Harrisburg, alternately in the title
/// and at the end of the summary.
pub fn search_corpus(store: &Store, n: usize) -> Vec<NewArticle> {
    const WORDS: &[&str] = &[
        "council", "budget", "storm", "election", "school", "river", "bridge", "court", "market",
        "transit", "hospital", "senate", "harbor", "museum", "festival", "factory",
    ];
    let sources: Vec<i64> = (0..50)
        .map(|i| {
            let url = format!("https://site{i}.example/feed");
            store
                .add_source(&NewSource {
                    folder_id: None,
                    kind: SourceKind::Feed,
                    title: url.clone(),
                    url: Some(url),
                    config: "{}".to_owned(),
                })
                .expect("feed")
                .id
        })
        .collect();
    (0..n)
        .map(|i| {
            let w = |k: usize| WORDS[(i * 7 + k * 13) % WORDS.len()];
            let nth = i / 2_700;
            let mention = i % 2_700 == 0 && nth < HARRISBURG;
            let title = if mention && nth % 2 == 0 {
                format!("Harrisburg {} {} update", w(0), w(1))
            } else {
                format!("{} {} report number {i}", w(0), w(1))
            };
            let tail = if mention && nth % 2 == 1 {
                " in Harrisburg."
            } else {
                "."
            };
            NewArticle {
                source_id: sources[i % sources.len()],
                guid: format!("urn:article:{i}"),
                url: Some(format!("https://example.com/{i}")),
                title,
                author: Some("A. Writer".to_owned()),
                summary_html: Some(format!(
                    "<p>The {} met over the {} and the {}{tail}</p>",
                    w(2),
                    w(3),
                    w(4)
                )),
                published_at: Some(1_700_000_000 + i as i64 * 60),
                fetched_at: 1_700_000_000 + i as i64 * 60 + 30,
            }
        })
        .collect()
}
