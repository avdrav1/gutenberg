//! MCP servers as sources: a command run as a child process that speaks MCP
//! on its stdin and stdout, or a URL that speaks streamable HTTP. Adding
//! one lists its tools; calling a tool on refresh is S6.2.

use std::process::Stdio;
use std::sync::Once;
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use process_wrap::tokio::{CommandWrap, ProcessGroup};
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::ClientInitializeError;
use rmcp::transport::streamable_http_client::StreamableHttpError;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// How long a server has to start and list its tools. A command run
/// through `npx` or `uvx` may first download the server.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Where a server is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Server {
    /// A program and its arguments, run without a shell.
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// A streamable HTTP endpoint.
    Url { url: String },
}

/// A tool a server offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    /// The first line of the server's description, if it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// An MCP source's settings, kept as JSON in `source.config`: the server,
/// and the tools it listed when last reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mcp {
    #[serde(flatten)]
    pub server: Server,
    #[serde(default)]
    pub tools: Vec<Tool>,
}

/// Why a server couldn't be reached or read, in one line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct McpError(String);

impl Server {
    /// A server from what the user typed: an `http://` or `https://` URL,
    /// or a command line, split the way a shell would (quotes kept
    /// together) but never run by one.
    pub fn parse(typed: &str) -> Result<Self, McpError> {
        let typed = typed.trim();
        if typed.starts_with("http://") || typed.starts_with("https://") {
            return url::Url::parse(typed)
                .map(|url| Server::Url { url: url.into() })
                .map_err(|e| McpError(format!("not a URL: {e}")));
        }
        let mut words = shlex::split(typed)
            .ok_or_else(|| McpError("a quote in the command isn't closed".into()))?
            .into_iter();
        let command = words
            .next()
            .ok_or_else(|| McpError("no command or URL".into()))?;
        Ok(Server::Command {
            command,
            args: words.collect(),
        })
    }

    /// What the source pane calls it until it's renamed: the URL, or the
    /// command line as typed.
    pub fn label(&self) -> String {
        match self {
            Server::Url { url } => url.clone(),
            Server::Command { command, args } => shlex::try_join(
                std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)),
            )
            .unwrap_or_else(|_| command.clone()),
        }
    }
}

impl Mcp {
    pub fn new(server: Server) -> Self {
        Self {
            server,
            tools: Vec::new(),
        }
    }

    /// The JSON for `source.config`.
    pub fn to_config(&self) -> String {
        serde_json::to_string(self).expect("strings and lists are JSON")
    }

    pub fn from_config(config: &str) -> Result<Self, McpError> {
        serde_json::from_str(config).map_err(|e| McpError(format!("MCP server settings: {e}")))
    }
}

/// Connects to `server`, asks for its tools and disconnects, within
/// `timeout`.
pub async fn tools(server: &Server, timeout: Duration) -> Result<Vec<Tool>, McpError> {
    let listed = match server {
        Server::Command { command, args } => {
            let mut cmd = Command::new(command);
            cmd.args(args);
            // Its own process group, so ending it ends whatever it started.
            let mut cmd = CommandWrap::from(cmd);
            cmd.wrap(ProcessGroup::leader());
            let (transport, stderr) = TokioChildProcess::builder(cmd)
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        McpError(format!("{command}: command not found"))
                    }
                    _ => McpError(format!("{command}: {e}")),
                })?;
            // What a server says on stderr before quitting is often the
            // only clue; keep its last line for the error.
            let said = tokio::spawn(last_line(stderr));
            let group = transport.id();
            let listed = tokio::time::timeout(timeout, list(transport)).await;
            // rmcp ends the server on a task that may never run, and only
            // the process it started; end the group, whatever happened.
            if let Some(leader) = group.and_then(|id| i32::try_from(id).ok()) {
                let _ = killpg(Pid::from_raw(leader), Signal::SIGKILL);
            }
            match listed {
                Ok(Err(e)) => {
                    let said = tokio::time::timeout(Duration::from_secs(1), said)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .flatten();
                    Ok(Err(match said {
                        Some(line) => McpError(format!("the server stopped: {line}")),
                        None => e,
                    }))
                }
                other => other,
            }
        }
        Server::Url { url } => {
            install_crypto();
            let transport = StreamableHttpClientTransport::from_uri(url.as_str());
            tokio::time::timeout(timeout, list(transport)).await
        }
    };
    listed.unwrap_or_else(|_| Err(McpError(format!("no answer in {} s", timeout.as_secs()))))
}

/// The handshake, the tool list, and a clean goodbye.
async fn list<T, E, A>(transport: T) -> Result<Vec<Tool>, McpError>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let me = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("gutenberg", env!("CARGO_PKG_VERSION")),
    );
    let client = me.serve(transport).await.map_err(|e| match &e {
        // The transport's own error holds the cause; the wrapper doesn't
        // chain to it.
        ClientInitializeError::TransportError { error, .. } => cause(error),
        _ => cause(&e),
    })?;
    let listed = client.peer().list_all_tools().await;
    let _ = client.cancel().await;
    let tools = listed.map_err(|e| cause(&e))?;
    Ok(tools
        .into_iter()
        .map(|t| Tool {
            name: t.name.into_owned(),
            description: t
                .description
                .as_deref()
                .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
                .map(str::to_owned),
        })
        .collect())
}

/// What went wrong at the bottom of an error's chain, such as "Connection
/// refused", rather than the transport types wrapped around it.
fn cause(error: &(dyn std::error::Error + 'static)) -> McpError {
    let mut deepest = error;
    loop {
        // rmcp's HTTP client error doesn't chain to reqwest's, which has
        // the reason, such as "Connection refused".
        let next = match deepest.downcast_ref::<StreamableHttpError<reqwest13::Error>>() {
            Some(StreamableHttpError::Client(e)) => Some(e as &(dyn std::error::Error + 'static)),
            _ => deepest.source(),
        };
        match next {
            Some(next) => deepest = next,
            None => break,
        }
    }
    let text = deepest.to_string();
    // An HTTP error carries the page after the status; the status says it.
    let text = text
        .strip_prefix("unexpected server response: ")
        .unwrap_or(&text);
    let text = match text.split_once(": ") {
        Some((status, _)) if status.starts_with("HTTP ") => status,
        _ => text,
    };
    McpError(one_line(text))
}

/// The last line with something on it, of at most 64 KiB read.
async fn last_line(stderr: Option<impl AsyncRead + Unpin>) -> Option<String> {
    let mut said = Vec::new();
    stderr?.take(64 * 1024).read_to_end(&mut said).await.ok()?;
    String::from_utf8_lossy(&said)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(one_line)
}

/// An error as one line of at most 200 characters.
fn one_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(text);
    let mut chars = line.chars();
    let short: String = chars.by_ref().take(200).collect();
    if chars.next().is_some() {
        format!("{short}…")
    } else {
        short
    }
}

/// The HTTP client rmcp builds uses rustls without a crypto provider of its
/// own; this one is the `ring` the feed client already uses.
fn install_crypto() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_split_like_a_shell_and_urls_stay_whole() {
        assert_eq!(
            Server::parse(r#" npx -y "@scope/some server" --dir '/tmp/a b' "#).unwrap(),
            Server::Command {
                command: "npx".into(),
                args: vec![
                    "-y".into(),
                    "@scope/some server".into(),
                    "--dir".into(),
                    "/tmp/a b".into()
                ],
            }
        );
        assert_eq!(
            Server::parse("https://mcp.example/v1?x=1").unwrap(),
            Server::Url {
                url: "https://mcp.example/v1?x=1".into()
            }
        );
        assert!(Server::parse("npx \"unclosed").is_err());
        assert!(Server::parse("  ").is_err());
    }

    #[test]
    fn settings_round_trip_and_label_the_server() {
        let mcp = Mcp {
            server: Server::parse("uvx arxiv-mcp-server --storage-path '/tmp/a b'").unwrap(),
            tools: vec![Tool {
                name: "search_papers".into(),
                description: Some("Search arXiv".into()),
            }],
        };
        let config = mcp.to_config();
        assert_eq!(Mcp::from_config(&config).unwrap(), mcp);
        assert_eq!(
            mcp.server.label(),
            "uvx arxiv-mcp-server --storage-path '/tmp/a b'"
        );
        let url = Mcp::from_config(r#"{"url":"https://mcp.example/v1"}"#).unwrap();
        assert_eq!(url.server.label(), "https://mcp.example/v1");
        assert!(url.tools.is_empty());
        assert!(Mcp::from_config("{}").is_err());
    }

    #[test]
    fn errors_are_one_short_line() {
        assert_eq!(one_line("\n  first\nsecond"), "first");
        assert_eq!(one_line(&"x".repeat(300)).chars().count(), 201);
    }
}
