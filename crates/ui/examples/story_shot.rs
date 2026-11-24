//! A story's screenshot, drawn headlessly in the current Omarchy theme (or
//! Tokyo Night): `s10-4`, the list in the interesting order, or `s10-5`,
//! the notification history.
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots \
//!     --example story_shot -- s10-5 docs/screenshots/s10-5 [store.db]
//! ```
//!
//! Writes `<out>-tiny-skia.png`. Made-up articles; `s10-4` takes a store
//! file instead, to check on real reading: a copy, since it's opened and
//! may be migrated.

use std::path::Path;

use gutenberg_theme::Tokens;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (story, out, store) = match args.as_slice() {
        [story, out] => (story.as_str(), out, None),
        [story, out, store] => (story.as_str(), out, Some(Path::new(store))),
        _ => usage(),
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let tokens = [
        "/.local/state/omarchy/current/theme/colors.toml",
        "/.config/omarchy/current/theme/colors.toml",
    ]
    .iter()
    .find_map(|p| std::fs::read_to_string(format!("{home}{p}")).ok())
    .and_then(|text| Tokens::from_colors_toml(&text).ok())
    .unwrap_or_else(|| {
        Tokens::from_colors_toml(include_str!(
            "../../../tests/fixtures/themes/tokyo-night.toml"
        ))
        .expect("built-in")
    });
    let (size, out) = ((1400.0, 720.0), Path::new(out));
    match (story, store) {
        ("s10-4", store) => gutenberg_ui::shots::render_interesting(tokens, size, store, out),
        ("s10-5", None) => gutenberg_ui::shots::render_history(tokens, size, out),
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!("usage: story_shot <s10-4 | s10-5> <output> [store.db, for s10-4]");
    std::process::exit(2);
}
