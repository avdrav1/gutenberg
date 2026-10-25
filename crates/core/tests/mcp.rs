//! S6.1: a refresh reaches each MCP server and keeps the tools it lists;
//! one it can't reach says why and backs off. The
//! server is the recorded Hugging Face one, played by
//! `tests/fixtures/mcp/replay.sh`.

use gutenberg_core::mcp::{self, Mcp, Server};
use gutenberg_core::sync::{refresh, Limits};
use gutenberg_store::Store;

const REPLAY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/mcp/replay.sh"
);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn tool_names(store: &Store, id: i64) -> Vec<String> {
    let source = store
        .sources()
        .unwrap()
        .into_iter()
        .find(|s| s.id == id)
        .unwrap();
    mcp::tools(&source).into_iter().map(|t| t.name).collect()
}

#[test]
fn a_refresh_keeps_each_servers_tools_and_errors() {
    let mut store = Store::open_in_memory().unwrap();
    let replay = Server::parse(&format!("sh {REPLAY}")).unwrap();
    let good = mcp::add(&store, &Mcp::new(replay), "Hugging Face", None)
        .unwrap()
        .unwrap()
        .id;
    let broken = Server::parse("sh -c 'echo Error: no token >&2; exit 1'").unwrap();
    let bad = mcp::add(&store, &Mcp::new(broken), "Broken", None)
        .unwrap()
        .unwrap()
        .id;

    let summary = runtime()
        .block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    assert_eq!(summary.sources, 2, "{summary}");
    assert_eq!(summary.failed.len(), 1, "{summary}");
    assert_eq!(
        tool_names(&store, good),
        ["hf_whoami", "hub_repo_search", "hub_repo_details", "hf_fs"]
    );
    assert!(store.fetch_state(good).unwrap().error.is_none());

    let state = store.fetch_state(bad).unwrap();
    assert_eq!(
        state.error.as_deref(),
        Some("the server stopped: Error: no token")
    );
    assert_eq!(state.failures, 1);
    assert!(state.retry_at.unwrap() > 1_000);
    assert_eq!(
        store.source_errors().unwrap().get(&bad).map(String::as_str),
        Some("the server stopped: Error: no token")
    );
    assert!(tool_names(&store, bad).is_empty());
}

#[test]
fn a_picked_tool_is_called_and_its_results_kept_once() {
    let mut store = Store::open_in_memory().unwrap();
    let mut papers =
        Mcp::new(Server::parse(&format!("sh {REPLAY} huggingface-hf-fs-trending.json")).unwrap());
    papers
        .pick(
            "hf_fs",
            r#"{"operations": [{"cmd": "ls", "args": ["hf://papers/trending", "--since", "{{since}}"]}]}"#,
        )
        .unwrap();
    let id = mcp::add(&store, &papers, "Trending papers", None)
        .unwrap()
        .unwrap()
        .id;
    let rt = runtime();
    // 2026-10-24 12:00 UTC.
    let noon = 1_792_843_200;
    let first = rt
        .block_on(refresh(&mut store, noon, &Limits::default(), None))
        .unwrap();
    assert_eq!((first.updated, first.new_articles), (1, 10), "{first}");
    let titles: Vec<_> = store
        .list_articles(Some(id), 20)
        .unwrap()
        .into_iter()
        .map(|a| a.title)
        .collect();
    assert!(titles
        .contains(&"Kronos: A Foundation Model for the Language of Financial Markets".to_owned()));
    let source = store.sources().unwrap().remove(0);
    let settings = mcp::settings(&source).unwrap();
    assert_eq!(settings.called.as_deref(), Some("2026-10-24"));
    assert_eq!(settings.tools.len(), 4);

    let second = rt
        .block_on(refresh(&mut store, noon + 3_600, &Limits::default(), None))
        .unwrap();
    assert_eq!(second.new_articles, 0, "{second}");
}
