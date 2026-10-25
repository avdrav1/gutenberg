//! A web page cut down to its article for the reader view.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let _ = gutenberg_sources::reader::extract(&html, "https://example.com/news/1");
});
