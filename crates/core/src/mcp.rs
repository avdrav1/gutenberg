//! MCP servers in the store: adding one, and the tools each listed when it
//! was last reached. The refresh reaches each (`sync`).

pub use gutenberg_sources::mcp::{Mcp, Server, Tool};
use gutenberg_store::{NewSource, Source, SourceKind, Store};

/// Adds `server`, named `title`, in `folder_id`. `None` when the store
/// already has it.
pub fn add(
    store: &Store,
    server: &Server,
    title: &str,
    folder_id: Option<i64>,
) -> gutenberg_store::Result<Option<Source>> {
    let known = store
        .sources()?
        .into_iter()
        .filter(|s| s.kind == SourceKind::Mcp)
        .filter_map(|s| Mcp::from_config(&s.config).ok())
        .any(|m| &m.server == server);
    if known {
        return Ok(None);
    }
    store
        .add_source(&NewSource {
            folder_id,
            kind: SourceKind::Mcp,
            title: title.to_owned(),
            url: None,
            config: Mcp::new(server.clone()).to_config(),
        })
        .map(Some)
}

/// The tools an MCP source listed when last reached; none for another
/// kind of source or one never reached.
pub fn tools(source: &Source) -> Vec<Tool> {
    match source.kind {
        SourceKind::Mcp => Mcp::from_config(&source.config)
            .map(|m| m.tools)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_added_once() {
        let store = Store::open_in_memory().unwrap();
        let server = Server::parse("uvx arxiv-mcp-server").unwrap();
        let added = add(&store, &server, "arXiv", None).unwrap().unwrap();
        assert_eq!(added.kind, SourceKind::Mcp);
        assert!(tools(&added).is_empty());
        // The same command, spaced differently, is the same server.
        let again = Server::parse("  uvx   arxiv-mcp-server ").unwrap();
        assert!(add(&store, &again, "arXiv", None).unwrap().is_none());
        let other = Server::parse("https://mcp.example/v1").unwrap();
        assert!(add(&store, &other, "Example", None).unwrap().is_some());
    }
}
