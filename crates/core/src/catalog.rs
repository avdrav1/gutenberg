//! The built-in catalog the Add sources screen offers: feeds by category,
//! keyword presets and MCP servers, from `catalog.toml`.

use std::collections::HashSet;
use std::sync::LazyLock;

use gutenberg_store::{NewSource, SourceKind, Store};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    #[serde(rename = "category")]
    pub categories: Vec<Category>,
    #[serde(rename = "feed")]
    pub feeds: Vec<CatalogFeed>,
    #[serde(rename = "keyword", default)]
    pub keywords: Vec<KeywordPreset>,
    #[serde(rename = "mcp", default)]
    pub mcp: Vec<McpPreset>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogFeed {
    /// A `Category::id`.
    pub category: String,
    pub title: String,
    pub url: String,
    pub site: String,
    pub blurb: String,
    pub paywall: Paywall,
    /// Checked on first run.
    #[serde(default)]
    pub starter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Paywall {
    None,
    Metered,
    Hard,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordPreset {
    pub category: String,
    pub title: String,
    pub query: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpPreset {
    pub title: String,
    pub blurb: String,
    pub transport: McpTransport,
    /// For `Stdio`.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// For `Http`.
    #[serde(default)]
    pub url: String,
    pub tool: String,
    /// The tool's arguments, a JSON object.
    pub arguments: String,
    /// The environment variable holding a key, if the server needs one.
    #[serde(default)]
    pub secret: String,
    pub homepage: String,
    pub maintainer: String,
}

static BUILTIN: LazyLock<Catalog> = LazyLock::new(|| {
    toml::from_str(include_str!("../catalog.toml")).expect("catalog.toml is checked by tests")
});

/// The catalog compiled into the binary, parsed on first use.
pub fn builtin() -> &'static Catalog {
    &BUILTIN
}

impl Catalog {
    /// A category's feeds in catalog order, with their index in `feeds`.
    pub fn feeds_in<'a>(
        &'a self,
        category: &'a str,
    ) -> impl Iterator<Item = (usize, &'a CatalogFeed)> + 'a {
        self.feeds
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.category == category)
    }

    /// A category's keyword presets, with their index in `keywords`.
    pub fn keywords_in<'a>(
        &'a self,
        category: &'a str,
    ) -> impl Iterator<Item = (usize, &'a KeywordPreset)> + 'a {
        self.keywords
            .iter()
            .enumerate()
            .filter(move |(_, k)| k.category == category)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Subscribed {
    /// New sources.
    pub added: usize,
    /// Picks whose URL the store already had.
    pub known: usize,
}

/// Adds the picked feeds (indices into `catalog.feeds`), each in a folder
/// named after its category. Feeds the store already has are skipped.
pub fn subscribe(
    store: &Store,
    catalog: &Catalog,
    picks: &[usize],
) -> gutenberg_store::Result<Subscribed> {
    let mut known: HashSet<String> = store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Feed)
        .filter_map(|s| s.url)
        .collect();
    let mut picks = picks.to_vec();
    picks.sort_unstable();
    picks.dedup();
    let mut result = Subscribed { added: 0, known: 0 };
    for feed in picks.iter().filter_map(|&i| catalog.feeds.get(i)) {
        if !known.insert(feed.url.clone()) {
            result.known += 1;
            continue;
        }
        let folder_id = match catalog.categories.iter().find(|c| c.id == feed.category) {
            Some(category) => Some(store.folder_named(&category.name)?.id),
            None => None,
        };
        store.add_source(&NewSource {
            folder_id,
            kind: SourceKind::Feed,
            title: feed.title.clone(),
            url: Some(feed.url.clone()),
            config: "{}".into(),
        })?;
        result.added += 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_holds_its_rules() {
        let catalog = builtin();
        let ids: HashSet<&str> = catalog.categories.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), catalog.categories.len(), "duplicate category id");
        for feed in &catalog.feeds {
            assert!(ids.contains(feed.category.as_str()), "{}", feed.title);
        }
        for keyword in &catalog.keywords {
            assert!(ids.contains(keyword.category.as_str()), "{}", keyword.title);
        }
        for category in &catalog.categories {
            let n = catalog.feeds_in(&category.id).count();
            assert!((8..=12).contains(&n), "{} has {n} feeds", category.name);
        }
        let mut urls = HashSet::new();
        for feed in &catalog.feeds {
            assert!(urls.insert(feed.url.as_str()), "duplicate {}", feed.url);
            for u in [&feed.url, &feed.site] {
                assert!(u.starts_with("https://"), "{u}");
                url::Url::parse(u).unwrap_or_else(|e| panic!("{u}: {e}"));
            }
            assert!(feed.title.chars().count() <= 40, "{}", feed.title);
            assert!(feed.blurb.chars().count() <= 70, "{}", feed.blurb);
        }
        let starters: Vec<_> = catalog.feeds.iter().filter(|f| f.starter).collect();
        assert_eq!(starters.len(), 12);
        assert!(starters.iter().all(|f| f.paywall == Paywall::None));
        for mcp in &catalog.mcp {
            assert!(mcp.arguments.starts_with('{'), "{}", mcp.title);
            match mcp.transport {
                McpTransport::Stdio => assert!(!mcp.command.is_empty(), "{}", mcp.title),
                McpTransport::Http => assert!(!mcp.url.is_empty(), "{}", mcp.title),
            }
        }
    }

    #[test]
    fn subscribe_files_feeds_by_category_and_skips_known() {
        let catalog = builtin();
        let store = Store::open_in_memory().unwrap();
        let world: Vec<usize> = catalog.feeds_in("world").map(|(i, _)| i).take(3).collect();
        let science = catalog.feeds_in("science").next().unwrap().0;
        store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "Already here".into(),
                url: Some(catalog.feeds[world[0]].url.clone()),
                config: "{}".into(),
            })
            .unwrap();

        let picks = [science, world[2], world[0], world[1], world[1]];
        let got = subscribe(&store, catalog, &picks).unwrap();
        assert_eq!(got, Subscribed { added: 3, known: 1 });
        let names: Vec<String> = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["World", "Science"]);
        assert_eq!(store.sources().unwrap().len(), 4);

        let again = subscribe(&store, catalog, &picks).unwrap();
        assert_eq!(again, Subscribed { added: 0, known: 4 });
        assert_eq!(store.folders().unwrap().len(), 2);
    }
}
