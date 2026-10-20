/// Where a source's articles come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Feed,
    Keyword,
    Mcp,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::Keyword => "keyword",
            Self::Mcp => "mcp",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "feed" => Some(Self::Feed),
            "keyword" => Some(Self::Keyword),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSource {
    pub folder_id: Option<i64>,
    pub kind: SourceKind,
    pub title: String,
    /// Feed or MCP URL; `None` for a keyword source.
    pub url: Option<String>,
    /// Kind-specific settings as JSON: query, providers, MCP command and tool.
    pub config: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub kind: SourceKind,
    pub title: String,
    pub url: Option<String>,
    pub config: String,
    pub position: i64,
    pub created_at: i64,
}

/// An article as a source fetched it. Times are unix seconds, UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewArticle {
    pub source_id: i64,
    /// The feed's GUID, or the URL when it has none.
    pub guid: String,
    pub url: Option<String>,
    pub title: String,
    pub author: Option<String>,
    pub summary_html: Option<String>,
    pub published_at: Option<i64>,
    pub fetched_at: i64,
}

/// What the article list shows. The summary is loaded on open with
/// [`Store::summary`](crate::Store::summary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRow {
    pub id: i64,
    pub source_id: i64,
    pub title: String,
    pub url: Option<String>,
    pub author: Option<String>,
    /// `published_at`, or `fetched_at` when the feed gave no date.
    pub sort_at: i64,
    pub read: bool,
    pub starred: bool,
}
