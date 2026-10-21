//! `gutenberg --import <file.opml>` and `gutenberg --export`: subscription
//! lists into and out of the store. Neither touches the network; imported
//! feeds fetch on the next refresh.

use std::collections::HashSet;
use std::process::ExitCode;

use gutenberg_sources::opml::{self, Folder, Opml, Subscription};
use gutenberg_store::{NewSource, SourceKind, Store};

pub fn import(path: &str) -> ExitCode {
    let result = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| opml::parse(&bytes).map_err(|e| e.to_string()))
        .and_then(|list| {
            let store = open()?;
            import_into(&store, &list).map_err(|e| e.to_string())
        });
    match result {
        Ok(imported) => {
            println!("{imported}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {path}: {message}");
            ExitCode::FAILURE
        }
    }
}

pub fn export() -> ExitCode {
    match open().and_then(|store| export_from(&store).map_err(|e| e.to_string())) {
        Ok(list) => {
            print!("{}", opml::write(&list));
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {message}");
            ExitCode::FAILURE
        }
    }
}

fn open() -> Result<Store, String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    Store::open(&path).map_err(|e| e.to_string())
}

/// What an import did, as the line it prints.
#[derive(Debug, PartialEq, Eq)]
struct Imported {
    added: usize,
    folders: usize,
    /// Feeds the store had before; they stay where they are.
    known: usize,
}

impl std::fmt::Display for Imported {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        write!(f, "Imported {}", plural(self.added, "feed", "feeds"))?;
        if self.folders > 0 {
            write!(f, " in {}", plural(self.folders, "folder", "folders"))?;
        }
        if self.known > 0 {
            write!(f, "; {} already added", self.known)?;
        }
        if self.added > 0 {
            write!(
                f,
                "\nArticles arrive with the next refresh, or now with gutenberg --refresh"
            )?;
        }
        Ok(())
    }
}

/// Every folder in the list is made (or reused by name), empty ones too. A
/// feed the list files in two folders lands in the first.
fn import_into(store: &Store, list: &Opml) -> gutenberg_store::Result<Imported> {
    let before: HashSet<String> = store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Feed)
        .filter_map(|s| s.url)
        .collect();
    let mut seen = HashSet::new();
    let mut imported = Imported {
        added: 0,
        folders: list.folders.len(),
        known: 0,
    };
    let mut targets = Vec::with_capacity(list.folders.len() + 1);
    for folder in &list.folders {
        targets.push((&folder.feeds, Some(store.folder_named(&folder.name)?.id)));
    }
    targets.push((&list.feeds, None));
    for (feeds, folder_id) in targets {
        for feed in feeds {
            if !seen.insert(feed.url.as_str()) {
                continue;
            }
            if before.contains(&feed.url) {
                imported.known += 1;
                continue;
            }
            store.add_source(&NewSource {
                folder_id,
                kind: SourceKind::Feed,
                title: feed.title.clone(),
                url: Some(feed.url.clone()),
                config: "{}".to_owned(),
            })?;
            imported.added += 1;
        }
    }
    Ok(imported)
}

/// Feeds in stored order, by folder. Keyword and MCP sources have no OPML
/// form and are left out.
fn export_from(store: &Store) -> gutenberg_store::Result<Opml> {
    let sources = store.sources()?;
    let feeds = |folder_id: Option<i64>| -> Vec<Subscription> {
        sources
            .iter()
            .filter(|s| s.kind == SourceKind::Feed && s.folder_id == folder_id)
            .filter_map(|s| {
                Some(Subscription {
                    title: s.title.clone(),
                    url: s.url.clone()?,
                })
            })
            .collect()
    };
    Ok(Opml {
        folders: store
            .folders()?
            .into_iter()
            .map(|f| Folder {
                feeds: feeds(Some(f.id)),
                name: f.name,
            })
            .collect(),
        feeds: feeds(None),
    })
}
