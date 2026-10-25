//! OPML, as `--import` and the Add sources screen read it. What parses
//! writes back out as OPML that parses again.
#![no_main]

use gutenberg_sources::opml;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(parsed) = opml::parse(data) {
        let written = opml::write(&parsed);
        if let Err(err) = opml::parse(written.as_bytes()) {
            panic!("written OPML doesn't parse: {err}\n{written}");
        }
    }
});
