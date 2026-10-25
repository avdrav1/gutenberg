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

/// How a new article arrives in the store: S8.4's rules may star it or
/// mark it read on the way in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Arrival {
    pub read: bool,
    pub starred: bool,
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
    /// The story it belongs to: the id of the story's first grouped
    /// article, or its own id while it's alone or not yet grouped.
    pub story_id: i64,
    /// How many articles the story has, this one included.
    pub story_size: i64,
}

/// An article as story grouping sees it, from
/// [`Store::story_candidates`](crate::Store::story_candidates).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryCandidate {
    pub id: i64,
    pub title: String,
    pub sort_at: i64,
    /// `None` until it's been grouped.
    pub story_id: Option<i64>,
}

/// What tells one of a source's articles from another, newest first from
/// [`Store::article_keys`](crate::Store::article_keys).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleKey {
    pub id: i64,
    pub guid: String,
    pub url: Option<String>,
    pub title: String,
}

/// HTTP validators sent back on the next fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validators {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// A source due for refresh, with what the last fetch left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueSource {
    pub source_id: i64,
    pub kind: SourceKind,
    /// A feed's URL; `None` for a keyword source, whose URL comes from
    /// `config`.
    pub url: Option<String>,
    pub config: String,
    pub validators: Validators,
    /// Failed refreshes in a row; drives backoff.
    pub failures: i64,
}

/// How a source's last refresh went.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchState {
    pub validators: Validators,
    pub checked_at: Option<i64>,
    /// One line; `None` when the last refresh worked.
    pub error: Option<String>,
    pub failures: i64,
    pub retry_at: Option<i64>,
}

/// Which articles the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Filter {
    All,
    Source(i64),
    Folder(i64),
}

/// Which of the filter's articles the list shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum View {
    #[default]
    All,
    Unread,
    Starred,
    /// Since midnight, local time.
    Today,
}

/// How many articles each view holds, across every feed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewCounts {
    pub unread: i64,
    pub starred: i64,
    pub today: i64,
}

/// Where a dragged source lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTarget {
    /// Just before this source, in its folder.
    Before(i64),
    /// After the last source in this folder, or unfiled when `None`.
    EndOf(Option<i64>),
}
