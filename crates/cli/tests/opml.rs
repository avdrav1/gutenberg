//! S2.5 acceptance: a Feedly export goes through `gutenberg --import` and
//! comes back out of `gutenberg --export` without losing a feed or folder.
//! Linux only: `XDG_DATA_HOME` points the store at a temp dir there.
#![cfg(target_os = "linux")]

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Output};

use gutenberg_sources::opml::{self, Opml};

const FEEDLY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/opml/feedly.opml"
);

fn gutenberg(data_home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutenberg"))
        .args(args)
        .env("XDG_DATA_HOME", data_home)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn import(data_home: &Path, file: &str) -> String {
    let out = gutenberg(data_home, &["--import", file]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn export(data_home: &Path) -> String {
    let out = gutenberg(data_home, &["--export"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    text(&out.stdout)
}

/// Every folder name, and every feed as (title, URL).
fn contents(list: &Opml) -> (Vec<&str>, HashSet<(&str, &str)>) {
    let folders = list.folders.iter().map(|f| f.name.as_str()).collect();
    let feeds = list
        .folders
        .iter()
        .flat_map(|f| &f.feeds)
        .chain(&list.feeds)
        .map(|s| (s.title.as_str(), s.url.as_str()))
        .collect();
    (folders, feeds)
}

#[test]
fn a_feedly_export_round_trips() {
    let data = tempfile::tempdir().unwrap();
    assert_eq!(
        import(data.path(), FEEDLY),
        "Imported 20 feeds in 6 folders\n\
         Articles arrive with the next refresh, or now with gutenberg --refresh\n"
    );
    let original = opml::parse(&std::fs::read(FEEDLY).unwrap()).unwrap();
    let exported_text = export(data.path());
    let exported = opml::parse(exported_text.as_bytes()).unwrap();
    assert_eq!(contents(&exported), contents(&original));

    // The feed Feedly filed twice sits in its first folder, once.
    let hn = "https://hnrss.org/frontpage?points=100&comments=25";
    let holding: Vec<&str> = exported
        .folders
        .iter()
        .filter(|f| f.feeds.iter().any(|s| s.url == hn))
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(holding, ["Programming"]);
    assert_eq!(exported.folders[2].feeds.len(), 4);
    // Folder and feed order survive too.
    assert_eq!(exported.folders[0].feeds, original.folders[0].feeds);

    // Our own export imports into a fresh store and comes out unchanged.
    let file = data.path().join("export.opml");
    std::fs::write(&file, &exported_text).unwrap();
    let fresh = tempfile::tempdir().unwrap();
    import(fresh.path(), file.to_str().unwrap());
    assert_eq!(export(fresh.path()), exported_text);
}

#[test]
fn importing_again_adds_nothing() {
    let data = tempfile::tempdir().unwrap();
    import(data.path(), FEEDLY);
    assert_eq!(
        import(data.path(), FEEDLY),
        "Imported 0 feeds in 6 folders; 20 already added\n"
    );
    let store = gutenberg_store::Store::open(&data.path().join("gutenberg/gutenberg.db")).unwrap();
    assert_eq!(store.sources().unwrap().len(), 20);
    assert_eq!(store.folders().unwrap().len(), 6);
}

#[test]
fn a_bad_file_is_one_line_of_error() {
    let data = tempfile::tempdir().unwrap();
    let page = data.path().join("page.html");
    std::fs::write(&page, "<html><body>not a list</body></html>").unwrap();
    let missing = data.path().join("missing.opml");
    for file in [&page, &missing] {
        let file = file.to_str().unwrap();
        let out = gutenberg(data.path(), &["--import", file]);
        assert!(!out.status.success());
        let stderr = text(&out.stderr);
        assert_eq!(stderr.lines().count(), 1, "{stderr}");
        assert!(
            stderr.starts_with(&format!("gutenberg: {file}: ")),
            "{stderr}"
        );
    }
    assert_eq!(
        text(&gutenberg(data.path(), &["--import", page.to_str().unwrap()]).stderr).trim(),
        format!(
            "gutenberg: {}: not OPML (no <opml> element)",
            page.display()
        )
    );
}
