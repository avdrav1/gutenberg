//! `gutenberg --unread [--json]`: how many stories are unread, the number
//! the window's Unread view shows, for a bar. Read from the store, so it
//! works whether or not the window is open. `--json` prints what a Waybar
//! custom module or an Omarchy shell command module takes.

use std::process::ExitCode;

use gutenberg_store::Store;

pub fn run(json: bool) -> ExitCode {
    match count() {
        Ok(unread) => {
            println!(
                "{}",
                if json {
                    as_json(unread)
                } else {
                    unread.to_string()
                }
            );
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: unread: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Unread stories; none before the store exists, which this doesn't create.
fn count() -> Result<i64, String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    if !path.exists() {
        return Ok(0);
    }
    let store = Store::open(&path).map_err(|e| e.to_string())?;
    unread(&store)
}

fn unread(store: &Store) -> Result<i64, String> {
    store
        .view_counts()
        .map(|counts| counts.unread)
        .map_err(|e| e.to_string())
}

/// Waybar's custom-module JSON. No text when nothing is unread, so a bar
/// can hide the module; `class` says which, for styling.
fn as_json(unread: i64) -> String {
    let (text, tooltip, class) = match unread {
        0 => (
            String::new(),
            "Nothing unread in Gutenberg".to_owned(),
            "read",
        ),
        1 => (
            "1".to_owned(),
            "1 unread story in Gutenberg".to_owned(),
            "unread",
        ),
        n => (
            n.to_string(),
            format!("{n} unread stories in Gutenberg"),
            "unread",
        ),
    };
    format!(r#"{{"text":"{text}","tooltip":"{tooltip}","class":"{class}","alt":"{class}"}}"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gutenberg_store::{NewArticle, NewSource, SourceKind};

    #[test]
    fn counts_unread_stories_once_each() {
        let mut store = Store::open_in_memory().unwrap();
        let feed = |title: &str| {
            store
                .add_source(&NewSource {
                    folder_id: None,
                    kind: SourceKind::Feed,
                    title: title.into(),
                    url: Some(format!("https://{title}.example/feed")),
                    config: "{}".into(),
                })
                .unwrap()
                .id
        };
        let (a, b) = (feed("a"), feed("b"));
        let article = |source_id, guid: &str| NewArticle {
            source_id,
            guid: guid.into(),
            url: None,
            title: guid.into(),
            author: None,
            summary_html: None,
            published_at: Some(1),
            fetched_at: 1,
        };
        store
            .insert_articles(&[article(a, "a1"), article(a, "a2"), article(b, "b1")])
            .unwrap();
        assert_eq!(unread(&store), Ok(3));
        // The same story from two feeds counts once.
        let id = |title: &str| {
            store
                .list_articles(None, 10)
                .unwrap()
                .into_iter()
                .find(|r| r.title == title)
                .unwrap()
                .id
        };
        let (a2, b1) = (id("a2"), id("b1"));
        store.set_stories(&[(b1, b1), (a2, b1)]).unwrap();
        assert_eq!(unread(&store), Ok(2));
    }

    #[test]
    fn json_is_what_waybar_takes() {
        assert_eq!(
            as_json(0),
            r#"{"text":"","tooltip":"Nothing unread in Gutenberg","class":"read","alt":"read"}"#
        );
        assert_eq!(
            as_json(1),
            r#"{"text":"1","tooltip":"1 unread story in Gutenberg","class":"unread","alt":"unread"}"#
        );
        let parsed: serde_json::Value = serde_json::from_str(&as_json(42)).unwrap();
        assert_eq!(parsed["text"], "42");
        assert_eq!(parsed["tooltip"], "42 unread stories in Gutenberg");
    }
}
