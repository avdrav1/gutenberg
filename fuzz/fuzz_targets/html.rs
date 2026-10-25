//! HTML from feeds: the article pane's parser, and the search index's
//! plain text. Search queries go through the same text code.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    gutenberg_ui::fuzz::article(&html);
    gutenberg_store::fuzz::plain(&html);
    gutenberg_store::fuzz::match_query(&html);
});
