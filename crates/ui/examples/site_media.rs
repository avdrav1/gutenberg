//! The site's screenshots and the trailers' frames, drawn headlessly from
//! the real window over made-up sample articles. `site/media/build.sh`
//! runs this, then cuts the trailers with ffmpeg.
//!
//! ```sh
//! ICED_TEST_BACKEND=tiny-skia cargo run -p gutenberg-ui --features shots \
//!     --example site_media -- tests/fixtures/themes <out dir>
//! ```
//!
//! Writes `<out>/shots/<name>.png`, `<out>/frames/<n>.png`, and
//! `<out>/short.txt` and `<out>/long.txt`: one line per frame, `<file>
//! <seconds> <caption>`, a caption of `-` for none.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gutenberg_theme::Tokens;
use gutenberg_ui::shots::Demo;

/// Screenshots: 16:10, the window's shape on a laptop.
const SHOT: (f32, f32) = (1280.0, 800.0);
/// Trailer frames: 16:9.
const FRAME: (f32, f32) = (1280.0, 720.0);

/// The themes the screenshots and trailers cycle through, dark and light.
const THEMES: [&str; 8] = [
    "tokyo-night",
    "catppuccin-latte",
    "gruvbox",
    "rose-pine",
    "everforest",
    "flexoki-light",
    "kanagawa",
    "nord",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [themes, out] = args.as_slice() else {
        eprintln!("usage: site_media <themes dir> <output dir>");
        std::process::exit(2);
    };
    let theme = |name: &str| {
        let path = Path::new(themes).join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path).expect("theme file");
        Tokens::from_colors_toml(&text).expect("theme")
    };
    let out = Path::new(out);
    for dir in ["shots", "frames"] {
        std::fs::create_dir_all(out.join(dir)).expect("output directory");
    }
    shots(&theme, &out.join("shots"));
    let mut reel = Reel::new(out.join("frames"));
    short(&theme, &mut reel);
    std::fs::write(out.join("short.txt"), reel.take()).expect("short.txt");
    long(&theme, &mut reel);
    std::fs::write(out.join("long.txt"), reel.take()).expect("long.txt");
}

fn shots(theme: &dyn Fn(&str) -> Tokens, out: &Path) {
    let dark = theme("tokyo-night");
    let mut demo = Demo::new(dark.clone(), SHOT);
    demo.shot(&out.join("reading"));
    demo.reader();
    demo.shot(&out.join("reader"));

    let mut demo = Demo::new(dark.clone(), SHOT);
    demo.search();
    "heat".chars().for_each(|c| demo.search_type(c));
    demo.shot(&out.join("search"));

    let mut demo = Demo::new(dark.clone(), SHOT);
    demo.palette();
    "st".chars().for_each(|c| demo.palette_type(c));
    demo.shot(&out.join("palette"));

    let mut demo = Demo::new(dark.clone(), SHOT);
    demo.help();
    demo.shot(&out.join("help"));

    let mut demo = Demo::first_run(dark.clone(), SHOT);
    demo.shot(&out.join("add-sources"));
    demo.new_query();
    "\"night trains\"".chars().for_each(|c| demo.query_type(c));
    demo.shot(&out.join("keyword"));

    for name in THEMES {
        let mut demo = Demo::new(theme(name), SHOT);
        demo.shot(&out.join(format!("theme-{name}")));
    }
}

/// Frames and how long each stays up.
struct Reel {
    dir: PathBuf,
    n: usize,
    lines: String,
}

impl Reel {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            n: 0,
            lines: String::new(),
        }
    }

    fn frame(&mut self, demo: &mut Demo, seconds: f32, caption: &str) {
        let name = format!("{:04}", self.n);
        self.n += 1;
        demo.shot(&self.dir.join(&name));
        let caption = if caption.is_empty() { "-" } else { caption };
        writeln!(self.lines, "{name}.png {seconds} {caption}").unwrap();
    }

    /// One frame per key, the way someone types.
    fn typing(&mut self, demo: &mut Demo, text: &str, key: fn(&mut Demo, char), caption: &str) {
        for c in text.chars() {
            key(demo, c);
            self.frame(demo, 0.09, caption);
        }
    }

    fn take(&mut self) -> String {
        std::mem::take(&mut self.lines)
    }
}

/// About 20 seconds, silent and captionless: the site's hero, on a loop.
fn short(theme: &dyn Fn(&str) -> Tokens, reel: &mut Reel) {
    let mut demo = Demo::new(theme("tokyo-night"), FRAME);
    reel.frame(&mut demo, 1.6, "");
    for _ in 0..3 {
        demo.down();
        reel.frame(&mut demo, 0.35, "");
    }
    demo.open();
    reel.frame(&mut demo, 1.0, "");
    demo.open_feature();
    reel.frame(&mut demo, 0.8, "");
    demo.reader();
    reel.frame(&mut demo, 2.2, "");
    demo.open_story();
    reel.frame(&mut demo, 1.6, "");
    demo.server();
    demo.open();
    reel.frame(&mut demo, 1.0, "");
    demo.tools();
    reel.frame(&mut demo, 1.4, "");
    demo.everything();
    demo.search();
    reel.frame(&mut demo, 0.3, "");
    reel.typing(&mut demo, "heat", Demo::search_type, "");
    reel.frame(&mut demo, 1.4, "");
    demo.end_search();
    demo.open_feature();
    for name in [
        "tokyo-night",
        "catppuccin-latte",
        "gruvbox",
        "rose-pine",
        "flexoki-light",
    ] {
        demo.set_tokens(theme(name));
        reel.frame(&mut demo, 1.0, "");
    }
    demo.set_tokens(theme("tokyo-night"));
    reel.frame(&mut demo, 0.8, "");
}

/// About seventy seconds, with a caption per feature.
fn long(theme: &dyn Fn(&str) -> Tokens, reel: &mut Reel) {
    let dark = theme("tokyo-night");

    let mut demo = Demo::first_run(dark.clone(), FRAME);
    let caption = "Start with 232 hand-picked feeds, or bring your own OPML";
    reel.frame(&mut demo, 2.4, caption);
    for _ in 0..2 {
        demo.sources_next();
        reel.frame(&mut demo, 0.7, caption);
    }
    for _ in 0..2 {
        demo.sources_down();
        reel.frame(&mut demo, 0.3, caption);
    }
    demo.sources_toggle();
    reel.frame(&mut demo, 1.2, caption);

    let caption = "Keyword alerts turn a news search into a source";
    demo.new_query();
    reel.frame(&mut demo, 0.8, caption);
    reel.typing(&mut demo, "\"night trains\"", Demo::query_type, caption);
    reel.frame(&mut demo, 1.6, caption);

    let mut demo = Demo::new(dark.clone(), FRAME);
    let caption = "Three panes, all keyboard: j and k to move, o to open";
    reel.frame(&mut demo, 2.0, caption);
    for _ in 0..4 {
        demo.down();
        reel.frame(&mut demo, 0.35, caption);
    }
    demo.open();
    reel.frame(&mut demo, 1.2, caption);
    demo.star();
    reel.frame(&mut demo, 1.2, "s stars it for later");
    demo.folder("Linux");
    reel.frame(
        &mut demo,
        1.4,
        "Folders and views keep unread counts honest",
    );
    demo.unread();
    reel.frame(
        &mut demo,
        1.4,
        "Folders and views keep unread counts honest",
    );
    demo.everything();
    demo.all();

    demo.open_story();
    reel.frame(
        &mut demo,
        3.0,
        "Eight outlets, one story: the list shows it once",
    );

    demo.open_feature();
    reel.frame(&mut demo, 1.6, "Feeds send a summary");
    demo.reader();
    reel.frame(
        &mut demo,
        3.2,
        "v fetches the whole story, without the clutter",
    );

    let caption = "MCP servers are sources too: pick a tool, read its answers";
    demo.server();
    demo.open();
    reel.frame(&mut demo, 1.8, caption);
    demo.tools();
    reel.frame(&mut demo, 1.4, caption);
    demo.down();
    reel.frame(&mut demo, 0.5, caption);
    demo.open();
    reel.frame(&mut demo, 0.8, caption);
    for _ in 0..r#"speech to text","repo_types":["model"],"limit":20}"#.len() {
        demo.arguments_back();
    }
    reel.frame(&mut demo, 0.3, caption);
    reel.typing(
        &mut demo,
        r#"text to speech","repo_types":["model"],"limit":20}"#,
        Demo::arguments_type,
        caption,
    );
    reel.frame(&mut demo, 1.4, caption);
    demo.arguments_close();
    demo.everything();
    demo.open_feature();

    let caption = "/ searches every article you have, as you type";
    demo.search();
    reel.frame(&mut demo, 0.5, caption);
    reel.typing(&mut demo, "heat pumps", Demo::search_type, caption);
    reel.frame(&mut demo, 2.0, caption);
    demo.end_search();

    let caption = ": runs any command by a few letters of its name";
    demo.palette();
    reel.frame(&mut demo, 0.6, caption);
    reel.typing(&mut demo, "mark", Demo::palette_type, caption);
    reel.frame(&mut demo, 1.6, caption);
    demo.close();
    demo.help();
    reel.frame(&mut demo, 2.4, "? shows every key");
    demo.close();

    demo.open_feature();
    let caption = "Colors and font follow your Omarchy theme, live";
    for name in THEMES {
        demo.set_tokens(theme(name));
        reel.frame(&mut demo, 1.1, caption);
    }
    demo.set_tokens(dark);
    reel.frame(&mut demo, 1.5, caption);
}
