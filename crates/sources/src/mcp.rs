//! MCP servers as sources: a command run as a child process that speaks MCP
//! on its stdin and stdout, or a URL that speaks streamable HTTP. Each
//! refresh lists the server's tools and, once one is picked, calls it with
//! its argument template; [`articles`] reads the result.

pub mod articles;

use std::process::Stdio;
use std::sync::Once;
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use process_wrap::tokio::{CommandWrap, ProcessGroup};
use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::ClientInitializeError;
use rmcp::transport::streamable_http_client::StreamableHttpError;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// How long a server has to start, list its tools and answer a call. A
/// command run through `npx` or `uvx` may first download the server.
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
    /// Arguments to start editing from, made from the tool's input schema:
    /// its required fields, each with its default or an empty value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
}

/// An MCP source's settings, kept as JSON in `source.config`: the server,
/// the tools it listed when last reached, and the tool to call on refresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mcp {
    #[serde(flatten)]
    pub server: Server,
    #[serde(default)]
    pub tools: Vec<Tool>,
    /// The tool called on refresh; until one is picked, a refresh only
    /// lists the tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The tool's arguments: a JSON object whose strings may hold
    /// `{{today}}` and `{{since}}` ([`fill`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    /// The day (`YYYY-MM-DD`, UTC) of the last call that succeeded, for
    /// `{{since}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub called: Option<String>,
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
            tool: None,
            arguments: None,
            called: None,
        }
    }

    /// Whether both call the same tool of the same server with the same
    /// arguments (as JSON, so spacing doesn't matter).
    pub fn same_call(&self, other: &Mcp) -> bool {
        let arguments = |m: &Mcp| {
            m.arguments
                .as_deref()
                .and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok())
        };
        self.server == other.server
            && self.tool == other.tool
            && arguments(self) == arguments(other)
    }

    /// Picks the tool to call on refresh and its argument template, which
    /// must be a JSON object once its placeholders are filled.
    pub fn pick(&mut self, tool: &str, arguments: &str) -> Result<(), McpError> {
        fill(arguments, "2000-01-01", "2000-01-01")?;
        self.tool = Some(tool.to_owned());
        self.arguments = Some(arguments.trim().to_owned());
        self.called = None;
        Ok(())
    }

    /// The JSON for `source.config`.
    pub fn to_config(&self) -> String {
        serde_json::to_string(self).expect("strings and lists are JSON")
    }

    pub fn from_config(config: &str) -> Result<Self, McpError> {
        serde_json::from_str(config).map_err(|e| McpError(format!("MCP server settings: {e}")))
    }
}

/// What one visit to a server got.
#[derive(Debug, Clone, PartialEq)]
pub struct Visit {
    pub tools: Vec<Tool>,
    /// The tool call's result as the server sent it (`content`,
    /// `structuredContent`, `isError`), when one was asked for.
    pub result: Option<serde_json::Value>,
}

/// Connects to `server`, asks for its tools and disconnects, within
/// `timeout`.
pub async fn tools(server: &Server, timeout: Duration) -> Result<Vec<Tool>, McpError> {
    visit(server, None, timeout).await.map(|v| v.tools)
}

/// Connects to `server`, lists its tools, calls `tool` with `arguments`
/// and disconnects, all within `timeout`.
pub async fn call(
    server: &Server,
    tool: &str,
    arguments: serde_json::Map<String, serde_json::Value>,
    timeout: Duration,
) -> Result<Visit, McpError> {
    visit(server, Some((tool, arguments)), timeout).await
}

type Call<'a> = Option<(&'a str, serde_json::Map<String, serde_json::Value>)>;

async fn visit(server: &Server, call: Call<'_>, timeout: Duration) -> Result<Visit, McpError> {
    let visited = match server {
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
            let visited = tokio::time::timeout(timeout, session(transport, call)).await;
            // rmcp ends the server on a task that may never run, and only
            // the process it started; end the group, whatever happened.
            if let Some(leader) = group.and_then(|id| i32::try_from(id).ok()) {
                let _ = killpg(Pid::from_raw(leader), Signal::SIGKILL);
            }
            match visited {
                Ok(Err(Failure::Connection(e))) => {
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
                Ok(Err(Failure::Tool(e))) => Ok(Err(e)),
                Ok(Ok(visit)) => Ok(Ok(visit)),
                Err(elapsed) => Err(elapsed),
            }
        }
        Server::Url { url } => {
            install_crypto();
            let transport = StreamableHttpClientTransport::from_uri(url.as_str());
            tokio::time::timeout(timeout, session(transport, call))
                .await
                .map(|r| r.map_err(Failure::into_error))
        }
    };
    visited.unwrap_or_else(|_| Err(McpError(format!("no answer in {} s", timeout.as_secs()))))
}

/// Whether a visit failed reaching the server, where what it printed
/// helps, or at the tool, where the server's own answer says it.
enum Failure {
    Connection(McpError),
    Tool(McpError),
}

impl Failure {
    fn into_error(self) -> McpError {
        match self {
            Failure::Connection(e) | Failure::Tool(e) => e,
        }
    }
}

/// The handshake, the tool list, the call if any, and a clean goodbye.
async fn session<T, E, A>(transport: T, call: Call<'_>) -> Result<Visit, Failure>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let me = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("gutenberg", env!("CARGO_PKG_VERSION")),
    );
    let client = me.serve(transport).await.map_err(|e| {
        Failure::Connection(match &e {
            // The transport's own error holds the cause; the wrapper
            // doesn't chain to it.
            ClientInitializeError::TransportError { error, .. } => cause(error),
            _ => cause(&e),
        })
    })?;
    let listed = client.peer().list_all_tools().await;
    let called = match (&listed, call) {
        (Ok(_), Some((tool, arguments))) => Some(
            client
                .peer()
                .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
                .await,
        ),
        _ => None,
    };
    let _ = client.cancel().await;
    let tools = listed.map_err(|e| Failure::Connection(cause(&e)))?;
    let result = match called {
        Some(Ok(result)) => Some(
            serde_json::to_value(&result)
                .map_err(|e| Failure::Tool(McpError(format!("the tool's result: {e}"))))?,
        ),
        Some(Err(e)) => return Err(Failure::Tool(cause(&e))),
        None => None,
    };
    Ok(Visit {
        tools: tools
            .into_iter()
            .map(|t| Tool {
                name: t.name.into_owned(),
                description: t
                    .description
                    .as_deref()
                    .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
                    .map(str::to_owned),
                example: Some(example(&t.input_schema)),
            })
            .collect(),
        result,
    })
}

/// Arguments for a tool from its input schema: each required field with
/// its default, its first example, or an empty value of its type.
pub fn example(schema: &serde_json::Map<String, serde_json::Value>) -> String {
    use serde_json::Value;
    let properties = schema.get("properties").and_then(Value::as_object);
    let required = schema.get("required").and_then(Value::as_array);
    let mut arguments = serde_json::Map::new();
    for name in required.into_iter().flatten().filter_map(Value::as_str) {
        let property = properties.and_then(|p| p.get(name));
        let field = |key: &str| property.and_then(|p| p.get(key));
        let value = field("default")
            .or_else(|| field("examples").and_then(|e| e.get(0)))
            .cloned()
            .unwrap_or_else(|| {
                let kind = field("type").and_then(|t| match t {
                    Value::Array(kinds) => kinds.iter().find_map(Value::as_str),
                    other => other.as_str(),
                });
                match kind {
                    Some("integer" | "number") => Value::from(0),
                    Some("boolean") => Value::Bool(false),
                    Some("array") => Value::Array(Vec::new()),
                    Some("object") => Value::Object(serde_json::Map::new()),
                    _ => Value::from(""),
                }
            });
        arguments.insert(name.to_owned(), value);
    }
    Value::Object(arguments).to_string()
}

/// An argument template filled in: `{{today}}` and `{{since}}` become
/// those days (`YYYY-MM-DD`), then the text must be a JSON object.
pub fn fill(
    template: &str,
    today: &str,
    since: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, McpError> {
    let text = template
        .replace("{{today}}", today)
        .replace("{{since}}", since);
    match serde_json::from_str(&text) {
        Ok(serde_json::Value::Object(arguments)) => Ok(arguments),
        Ok(_) => Err(McpError("the arguments must be a JSON object, {…}".into())),
        Err(e) => Err(McpError(format!("the arguments aren't JSON: {e}"))),
    }
}

/// A Unix time's day, `YYYY-MM-DD`, in UTC.
pub fn day(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .format("%Y-%m-%d")
        .to_string()
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
pub(crate) fn one_line(text: &str) -> String {
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
        let mut mcp = Mcp {
            tools: vec![Tool {
                name: "search_papers".into(),
                description: Some("Search arXiv".into()),
                example: Some(r#"{"query":""}"#.into()),
            }],
            ..Mcp::new(Server::parse("uvx arxiv-mcp-server --storage-path '/tmp/a b'").unwrap())
        };
        mcp.pick(
            "search_papers",
            r#" {"query": "agents", "since": "{{since}}"} "#,
        )
        .unwrap();
        assert_eq!(
            mcp.arguments.as_deref(),
            Some(r#"{"query": "agents", "since": "{{since}}"}"#)
        );
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
    fn templates_fill_their_days_and_must_be_objects() {
        let filled = fill(
            r#"{"q": "news", "from": "{{since}}", "to": "{{today}}", "n": 5}"#,
            "2026-10-24",
            "2026-10-23",
        )
        .unwrap();
        assert_eq!(
            serde_json::Value::Object(filled),
            serde_json::json!({"q": "news", "from": "2026-10-23", "to": "2026-10-24", "n": 5})
        );
        assert!(fill("[1, 2]", "", "").is_err());
        assert!(fill("{\"q\": ", "", "").is_err());
        let mut mcp = Mcp::new(Server::parse("x").unwrap());
        assert!(mcp.pick("t", "not json").is_err());
        assert_eq!(mcp.tool, None);
        assert_eq!(day(1_792_828_800), "2026-10-24");
    }

    #[test]
    fn examples_hold_the_required_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "count": {"type": "integer", "default": 20},
                "sources": {"type": ["array", "null"]},
                "lang": {"type": "string", "examples": ["en"]},
                "optional": {"type": "string"}
            },
            "required": ["query", "count", "sources", "lang"]
        });
        let example = example(schema.as_object().unwrap());
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&example).unwrap(),
            serde_json::json!({"query": "", "count": 20, "sources": [], "lang": "en"})
        );
        assert_eq!(super::example(&serde_json::Map::new()), "{}");
    }

    #[test]
    fn errors_are_one_short_line() {
        assert_eq!(one_line("\n  first\nsecond"), "first");
        assert_eq!(one_line(&"x".repeat(300)).chars().count(), 201);
    }
}
