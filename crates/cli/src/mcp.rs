//! `gutenberg --mcp <command or URL> [--tool <name> [--args <json>]]
//! [--secret <NAME>]`: add an MCP server, list its tools, and with `--tool`
//! read its results as articles on every refresh. `--secret` names the key
//! it needs, from the environment or the keyring (`--key`).

use std::process::ExitCode;

use gutenberg_core::{mcp, sync};
use gutenberg_store::Store;

/// What follows `--mcp`.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub server: String,
    pub tool: Option<String>,
    /// The tool's argument template; `{}` when not given.
    pub arguments: Option<String>,
    /// The name of the key the server needs.
    pub secret: Option<String>,
    pub folder: Option<String>,
}

impl Options {
    /// The server, then `--tool`, `--args`, `--secret` and `--folder` in
    /// any order.
    /// `None` for anything else, or `--args` without `--tool`.
    pub fn parse(args: &[String]) -> Option<Self> {
        let (server, mut rest) = args.split_first()?;
        if server.starts_with("--") {
            return None;
        }
        let mut options = Options {
            server: server.clone(),
            tool: None,
            arguments: None,
            secret: None,
            folder: None,
        };
        while let [flag, value, more @ ..] = rest {
            let slot = match flag.as_str() {
                "--tool" => &mut options.tool,
                "--args" => &mut options.arguments,
                "--secret" => &mut options.secret,
                "--folder" => &mut options.folder,
                _ => return None,
            };
            if slot.replace(value.clone()).is_some() {
                return None;
            }
            rest = more;
        }
        (rest.is_empty() && (options.tool.is_some() || options.arguments.is_none()))
            .then_some(options)
    }
}

pub fn run(options: &Options) -> ExitCode {
    match add(options) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {}: {message}", options.server);
            ExitCode::FAILURE
        }
    }
}

fn add(options: &Options) -> Result<String, String> {
    let mut settings =
        mcp::Mcp::new(mcp::Server::parse(&options.server).map_err(|e| e.to_string())?);
    if let Some(tool) = &options.tool {
        settings
            .pick(tool, options.arguments.as_deref().unwrap_or("{}"))
            .map_err(|e| e.to_string())?;
    }
    if let Some(name) = &options.secret {
        if !gutenberg_core::secrets::valid_name(name) {
            return Err(format!(
                "--secret {name}: a key's name is letters, digits and _, like JINA_API_KEY"
            ));
        }
        settings.secret = Some(name.clone());
    }
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let folder_id = match &options.folder {
        Some(name) => Some(store.folder_named(name).map_err(|e| e.to_string())?.id),
        None => None,
    };
    let label = match &options.tool {
        Some(tool) => format!("{} · {tool}", settings.server.label()),
        None => settings.server.label(),
    };
    let source = mcp::add(&store, &settings, &label, folder_id)
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
            &crate::limits(),
        ))
        .map_err(|e| e.to_string())?;
    // A server that can't be reached stays, and is tried again on the next
    // refresh; the source pane shows why.
    if let Some((_, error)) = summary.failed.first() {
        let hint = match &settings.secret {
            Some(name) if error == &sync::missing_key(name) => {
                format!("; give it {name} with: gutenberg --key {name}")
            }
            _ => String::new(),
        };
        return Ok(format!(
            "Added MCP server {label}; couldn't reach it ({error}){hint}"
        ));
    }
    let source = store
        .sources()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.id == source.id)
        .ok_or("the server's source went missing")?;
    let tools = mcp::tools(&source);
    let place = options
        .folder
        .as_ref()
        .map_or(String::new(), |name| format!(" to {name}"));
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    let mut message = match &options.tool {
        Some(tool) => format!(
            "Added MCP server {label}{place}: {} from {tool}",
            plural(summary.new_articles, "article")
        ),
        None => format!(
            "Added MCP server {label}{place}: {}",
            plural(tools.len(), "tool")
        ),
    };
    if options.tool.is_none() {
        for tool in tools {
            message.push_str(&format!("\n  {}", tool.name));
            if let Some(description) = tool.description {
                message.push_str(&format!(" · {description}"));
            }
        }
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Options> {
        Options::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn flags_come_in_any_order_once_each() {
        let options = parse(&[
            "uvx x", "--args", "{}", "--folder", "AI", "--secret", "X_KEY", "--tool", "t",
        ])
        .unwrap();
        assert_eq!(
            options,
            Options {
                server: "uvx x".into(),
                tool: Some("t".into()),
                arguments: Some("{}".into()),
                secret: Some("X_KEY".into()),
                folder: Some("AI".into()),
            }
        );
        assert!(parse(&["uvx x"]).is_some());
        assert!(parse(&[]).is_none());
        assert!(parse(&["--tool", "t"]).is_none());
        assert!(parse(&["uvx x", "--args", "{}"]).is_none());
        assert!(parse(&["uvx x", "--tool"]).is_none());
        assert!(parse(&["uvx x", "--tool", "a", "--tool", "b"]).is_none());
        assert!(parse(&["uvx x", "--color", "red"]).is_none());
    }
}
