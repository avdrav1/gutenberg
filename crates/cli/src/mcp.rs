//! `gutenberg --mcp <command or URL>`: add an MCP server and list its tools.

use std::process::ExitCode;

use gutenberg_core::{mcp, sync};
use gutenberg_store::Store;

pub fn run(typed: &str, folder: Option<&str>) -> ExitCode {
    match add(typed, folder) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {typed}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn add(typed: &str, folder: Option<&str>) -> Result<String, String> {
    let server = mcp::Server::parse(typed).map_err(|e| e.to_string())?;
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let folder_id = match folder {
        Some(name) => Some(store.folder_named(name).map_err(|e| e.to_string())?.id),
        None => None,
    };
    let label = server.label();
    let source = mcp::add(&store, &server, &label, folder_id)
        .map_err(|e| e.to_string())?
        .ok_or("already added")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let summary = runtime
        .block_on(sync::refresh_source(
            &mut store,
            source.id,
            sync::unix_now(),
            &sync::Limits::default(),
        ))
        .map_err(|e| e.to_string())?;
    // A server that can't be reached stays, and is tried again on the next
    // refresh; the source pane shows why.
    if let Some((_, error)) = summary.failed.first() {
        return Ok(format!(
            "Added MCP server {label}; couldn't reach it ({error})"
        ));
    }
    let source = store
        .sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == source.id)
        .ok_or("the server's source went missing")?;
    let tools = mcp::tools(&source);
    let place = folder.map_or(String::new(), |name| format!(" to {name}"));
    let noun = if tools.len() == 1 { "tool" } else { "tools" };
    let mut message = format!("Added MCP server {label}{place}: {} {noun}", tools.len());
    for tool in tools {
        message.push_str(&format!("\n  {}", tool.name));
        if let Some(description) = tool.description {
            message.push_str(&format!(" · {description}"));
        }
    }
    Ok(message)
}
