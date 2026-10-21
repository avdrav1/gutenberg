//! Rechecks every feed in `catalog.toml` against the live web: it must
//! fetch, parse, have an entry from the last 60 days and at most 300
//! entries. Network; run by hand before a release, never by `cargo test`.
//!
//! cargo run -p gutenberg-core --example check_catalog

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::stream::{self, StreamExt};
use gutenberg_core::catalog::{self, CatalogFeed};
use gutenberg_sources::feed::{client, fetch_conditional, parse, Client, Fetched};

const MAX_AGE_DAYS: i64 = 60;
const MAX_ENTRIES: usize = 300;

fn main() -> ExitCode {
    let catalog = catalog::builtin();
    let client = match client() {
        Ok(client) => client,
        Err(e) => {
            eprintln!("check_catalog: {e}");
            return ExitCode::FAILURE;
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let problems: Vec<String> = runtime.block_on(
        stream::iter(&catalog.feeds)
            .map(|feed| check(&client, feed, now))
            .buffer_unordered(16)
            .filter_map(|problem| async move { problem })
            .collect(),
    );
    for problem in &problems {
        println!("{problem}");
    }
    let total = catalog.feeds.len();
    let bad = problems.len();
    println!("{total} feeds: {} ok, {bad} problems", total - bad);
    if bad > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// One line describing what's wrong with the feed, or `None` when it's fine.
async fn check(client: &Client, feed: &CatalogFeed, now: i64) -> Option<String> {
    let name = format!("{}/{}", feed.category, feed.title);
    let bytes = match fetch_conditional(client, &feed.url, None, None).await {
        Ok(Fetched::Body { bytes, .. }) => bytes,
        Ok(Fetched::NotModified) => return Some(format!("FAIL {name}: 304 without validators")),
        Err(e) => return Some(format!("FAIL {name}: {e}")),
    };
    let parsed = match parse(&bytes, &feed.url) {
        Ok(parsed) => parsed,
        Err(e) => return Some(format!("FAIL {name}: {e}")),
    };
    let newest = parsed.entries.iter().filter_map(|e| e.published_at).max();
    match newest {
        None => return Some(format!("STALE {name}: no dated entries")),
        Some(t) if now - t > MAX_AGE_DAYS * 86_400 => {
            let (y, m, d) = civil(t.div_euclid(86_400));
            return Some(format!("STALE {name}: newest {y:04}-{m:02}-{d:02}"));
        }
        Some(_) => {}
    }
    let n = parsed.entries.len();
    (n > MAX_ENTRIES).then(|| format!("BIG {name}: {n} entries"))
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, i64, i64) {
    // Howard Hinnant's days_from_civil, inverted.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
