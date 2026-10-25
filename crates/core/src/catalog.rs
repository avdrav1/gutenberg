//! The built-in catalog the Add sources screen offers: feeds by category,
//! keyword presets and MCP servers, from `catalog.toml`.

use std::collections::HashSet;
use std::sync::LazyLock;

use gutenberg_sources::keyword::Keyword;
use gutenberg_store::{NewSource, SourceKind, Store};
use serde::Deserialize;

use crate::keyword;
use crate::mcp::{self, Mcp, Server};

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

/// What the Add sources screen picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picks {
    /// Indices into `catalog.feeds`.
    pub feeds: Vec<usize>,
    /// Indices into `catalog.keywords`.
    pub keywords: Vec<usize>,
    /// Keyword alerts typed in, as `Keyword::new` keeps them.
    pub queries: Vec<String>,
    /// Indices into `catalog.mcp`.
    pub mcp: Vec<usize>,
}

impl Picks {
    pub fn len(&self) -> usize {
        self.feeds.len() + self.keywords.len() + self.queries.len() + self.mcp.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Subscribed {
    /// New sources.
    pub added: usize,
    /// Picks the store already had: a feed's URL, an alert's query.
    pub known: usize,
}

/// Adds the picks. Catalog feeds and keyword presets go in a folder named
/// after their category; typed-in queries go at the top level, titled by
/// the query. What the store already has is skipped.
pub fn subscribe(
    store: &Store,
    catalog: &Catalog,
    picks: &Picks,
) -> gutenberg_store::Result<Subscribed> {
    let mut known: HashSet<String> = store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Feed)
        .filter_map(|s| s.url)
        .collect();
    let mut result = Subscribed { added: 0, known: 0 };
    let folder = |category: &str| -> gutenberg_store::Result<Option<i64>> {
        match catalog.categories.iter().find(|c| c.id == category) {
            Some(category) => Ok(Some(store.folder_named(&category.name)?.id)),
            None => Ok(None),
        }
    };
    for feed in sorted(&picks.feeds).filter_map(|i| catalog.feeds.get(i)) {
        if !known.insert(feed.url.clone()) {
            result.known += 1;
            continue;
        }
        store.add_source(&NewSource {
            folder_id: folder(&feed.category)?,
            kind: SourceKind::Feed,
            title: feed.title.clone(),
            url: Some(feed.url.clone()),
            config: "{}".into(),
        })?;
        result.added += 1;
    }
    let presets = sorted(&picks.keywords).filter_map(|i| catalog.keywords.get(i));
    let alerts = presets
        .map(|k| {
            (
                k.query.as_str(),
                k.title.as_str(),
                Some(k.category.as_str()),
            )
        })
        .chain(picks.queries.iter().map(|q| (q.as_str(), q.as_str(), None)));
    let mut queries = keyword::queries(store)?;
    for (query, title, category) in alerts {
        let Some(keyword) = Keyword::new(query) else {
            continue;
        };
        if !queries.insert(keyword.query.clone()) {
            result.known += 1;
            continue;
        }
        let folder_id = match category {
            Some(category) => folder(category)?,
            None => None,
        };
        keyword::insert(store, &keyword, title, folder_id)?;
        result.added += 1;
    }
    for preset in sorted(&picks.mcp).filter_map(|i| catalog.mcp.get(i)) {
        match mcp::add(store, &preset.settings(), &preset.title, None)? {
            Some(_) => result.added += 1,
            None => result.known += 1,
        }
    }
    Ok(result)
}

impl McpPreset {
    /// The source settings for this preset: its server, tool and arguments.
    pub fn settings(&self) -> Mcp {
        let server = match self.transport {
            McpTransport::Stdio => Server::Command {
                command: self.command.clone(),
                args: self.args.clone(),
            },
            McpTransport::Http => Server::Url {
                url: self.url.clone(),
            },
        };
        let mut settings = Mcp::new(server);
        settings
            .pick(&self.tool, &self.arguments)
            .expect("the catalog's arguments are JSON objects (tested)");
        settings.secret = (!self.secret.is_empty()).then(|| self.secret.clone());
        settings
    }
}

/// Each index once, in order.
fn sorted(indices: &[usize]) -> impl Iterator<Item = usize> {
    let mut indices = indices.to_vec();
    indices.sort_unstable();
    indices.dedup();
    indices.into_iter()
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
        let mut queries = HashSet::new();
        for keyword in &catalog.keywords {
            assert!(ids.contains(keyword.category.as_str()), "{}", keyword.title);
            let query = Keyword::new(&keyword.query).expect("a query").query;
            assert!(queries.insert(query), "duplicate {}", keyword.query);
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
            // The template is a JSON object once filled, and a key's name
            // comes along, never a key.
            let settings = mcp.settings();
            if !mcp.secret.is_empty() {
                assert!(crate::secrets::valid_name(&mcp.secret), "{}", mcp.title);
                assert_eq!(settings.secret.as_deref(), Some(mcp.secret.as_str()));
            }
            match mcp.transport {
                McpTransport::Stdio => assert!(!mcp.command.is_empty(), "{}", mcp.title),
                McpTransport::Http => assert!(!mcp.url.is_empty(), "{}", mcp.title),
            }
        }
    }

    #[test]
    fn subscribe_files_by_category_and_skips_known() {
        let catalog = builtin();
        let store = Store::open_in_memory().unwrap();
        let world: Vec<usize> = catalog.feeds_in("world").map(|(i, _)| i).take(3).collect();
        let science = catalog.feeds_in("science").next().unwrap().0;
        let un = catalog.keywords_in("world").next().unwrap().0;
        store
            .add_source(&NewSource {
                folder_id: None,
                kind: SourceKind::Feed,
                title: "Already here".into(),
                url: Some(catalog.feeds[world[0]].url.clone()),
                config: "{}".into(),
            })
            .unwrap();

        let papers = catalog
            .mcp
            .iter()
            .position(|m| m.title == "Hugging Face – Daily Papers")
            .unwrap();
        let picks = Picks {
            feeds: vec![science, world[2], world[0], world[1], world[1]],
            keywords: vec![un, un],
            // The second is the preset's query, spaced differently.
            queries: vec![
                "\"Cory Booker\"".into(),
                format!(" {} ", catalog.keywords[un].query),
            ],
            mcp: vec![papers, papers],
        };
        let got = subscribe(&store, catalog, &picks).unwrap();
        assert_eq!(got, Subscribed { added: 6, known: 2 });
        // An MCP server comes with its tool, unfiled; added again, it's known.
        let server = store
            .sources()
            .unwrap()
            .into_iter()
            .find(|s| s.kind == SourceKind::Mcp)
            .unwrap();
        assert_eq!(server.title, catalog.mcp[papers].title);
        assert_eq!(server.folder_id, None);
        let settings = mcp::settings(&server).unwrap();
        assert_eq!(settings.tool.as_deref(), Some("hf_fs"));
        let again = Picks {
            mcp: vec![papers],
            ..Picks::default()
        };
        assert_eq!(
            subscribe(&store, catalog, &again).unwrap(),
            Subscribed { added: 0, known: 1 }
        );
        let names: Vec<String> = store
            .folders()
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["World", "Science"]);
        let sources = store.sources().unwrap();
        let alerts: Vec<(&str, bool)> = sources
            .iter()
            .filter(|s| s.kind == SourceKind::Keyword)
            .map(|s| (s.title.as_str(), s.folder_id.is_some()))
            .collect();
        assert_eq!(
            alerts,
            [
                (catalog.keywords[un].title.as_str(), true),
                ("\"Cory Booker\"", false)
            ]
        );

        let again = subscribe(&store, catalog, &picks).unwrap();
        assert_eq!(again, Subscribed { added: 0, known: 8 });
        assert_eq!(store.folders().unwrap().len(), 2);
    }
}
