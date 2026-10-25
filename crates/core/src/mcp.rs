//! MCP servers in the store: adding one, picking the tool its refresh
//! calls, and the tools each listed when it was last reached. The refresh
//! reaches each (`sync`).

pub use gutenberg_sources::mcp::{Mcp, McpError, Server, Tool};
use gutenberg_store::{NewSource, Source, SourceKind, Store};

/// Adds `mcp`, named `title`, in `folder_id`. `None` when the store
/// already has that server calling the same tool with the same arguments.
pub fn add(
    store: &Store,
    mcp: &Mcp,
    title: &str,
    folder_id: Option<i64>,
) -> gutenberg_store::Result<Option<Source>> {
    let known = store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Mcp)
        .filter_map(|s| Mcp::from_config(&s.config).ok())
        .any(|m| m.same_call(mcp));
    if known {
        return Ok(None);
    }
    store
        .add_source(&NewSource {
            folder_id,
            kind: SourceKind::Mcp,
            title: title.to_owned(),
            url: None,
            config: mcp.to_config(),
        })
        .map(Some)
}

/// Picks the tool a server's refresh calls and its argument template, and
/// has the next refresh reach it now.
pub fn pick(store: &Store, source: &Source, tool: &str, arguments: &str) -> Result<(), String> {
    let mut mcp = Mcp::from_config(&source.config).map_err(|e| e.to_string())?;
    mcp.pick(tool, arguments).map_err(|e| e.to_string())?;
    store
        .set_source_config(source.id, &mcp.to_config())
        .and_then(|()| store.recheck(source.id))
        .map_err(|e| e.to_string())
}

/// An MCP source's settings; `None` for another kind of source or ones
/// that can't be read.
pub fn settings(source: &Source) -> Option<Mcp> {
    match source.kind {
        SourceKind::Mcp => Mcp::from_config(&source.config).ok(),
        _ => None,
    }
}

/// The tools an MCP source listed when last reached; none for another
/// kind of source or one never reached.
pub fn tools(source: &Source) -> Vec<Tool> {
    settings(source).map(|m| m.tools).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_and_tool_are_added_once() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::parse("uvx arxiv-mcp-server").unwrap();
        let added = add(&store, &Mcp::new(server.clone()), "arXiv", None)
            .unwrap()
            .unwrap();
        assert_eq!(added.kind, SourceKind::Mcp);
        assert!(tools(&added).is_empty());
        // The same command, spaced differently, is the same server.
        let again = Mcp::new(Server::parse("  uvx   arxiv-mcp-server ").unwrap());
        assert!(add(&store, &again, "arXiv", None).unwrap().is_none());
        // Another tool of it is another source, and so is the same tool
        // with other arguments.
        let mut search = Mcp::new(server);
        search
            .pick("search_papers", r#"{"query": "agents"}"#)
            .unwrap();
        assert!(add(&store, &search, "arXiv agents", None)
            .unwrap()
            .is_some());
        assert!(add(&store, &search, "arXiv agents", None)
            .unwrap()
            .is_none());
        search
            .pick("search_papers", r#"{"query": "robots"}"#)
            .unwrap();
        assert!(add(&store, &search, "arXiv robots", None)
            .unwrap()
            .is_some());
    }

    #[test]
    fn picking_a_tool_saves_it_and_rechecks_the_server() {
        let store = Store::open_in_memory().unwrap();
        let server = Mcp::new(Server::parse("https://mcp.example/v1").unwrap());
        let source = add(&store, &server, "Example", None).unwrap().unwrap();
        store.record_failure(source.id, "HTTP 500", 999, 1).unwrap();
        assert!(pick(&store, &source, "search", "not json").is_err());
        pick(&store, &source, "search", r#"{"q": "{{today}}"}"#).unwrap();
        let picked = settings(&store.sources().unwrap()[0]).unwrap();
        assert_eq!(picked.tool.as_deref(), Some("search"));
        assert!(store.has_unchecked_sources().unwrap());
        assert_eq!(store.fetch_state(source.id).unwrap().error, None);
    }
}
