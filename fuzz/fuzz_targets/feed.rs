//! RSS, Atom and JSON Feed, as a refresh gets them: any bytes parse to a
//! feed or an error, never a panic.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = gutenberg_sources::feed::parse(data, "https://example.com/feed.xml");
});
