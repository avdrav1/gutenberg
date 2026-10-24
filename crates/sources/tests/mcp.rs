//! S6.1: an MCP server, by command or URL, lists its tools; one that can't
//! be reached says why in one line. The server is the Hugging Face one
//! recorded in `tests/fixtures/mcp/`, played over stdio by `replay.sh` or
//! over HTTP by a local server on 127.0.0.1; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gutenberg_sources::mcp::{self, Server, Tool};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/mcp");

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn tools(server: &Server) -> Result<Vec<Tool>, String> {
    runtime()
        .block_on(mcp::tools(server, Duration::from_secs(10)))
        .map_err(|e| e.to_string())
}

fn command(line: &str) -> Server {
    Server::parse(line).unwrap()
}

/// The four tools, as `tools/list` recorded them: names, and the first
/// line of each description.
fn assert_recorded(tools: &[Tool]) {
    let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        ["hf_whoami", "hub_repo_search", "hub_repo_details", "hf_fs"]
    );
    assert!(tools[1]
        .description
        .as_deref()
        .unwrap()
        .starts_with("Search Hugging Face repositories with a shared query interface."));
    assert!(tools
        .iter()
        .all(|t| !t.description.as_deref().unwrap_or_default().contains('\n')));
}

#[test]
fn a_command_lists_its_tools() {
    let listed = tools(&command(&format!("sh {FIXTURES}/replay.sh"))).unwrap();
    assert_recorded(&listed);
}

/// Requests a local server got: method, and the session header if any.
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

/// The recorded server over streamable HTTP, answering as Hugging Face did.
fn serve() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let reply = |name: &str| std::fs::read_to_string(format!("{FIXTURES}/{name}")).unwrap();
    let (init, list) = (
        reply("huggingface-initialize.json"),
        reply("huggingface-tools-list.json"),
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let (method, session, body) = read_request(&mut stream);
            log.lock().unwrap().push((method.clone(), session));
            let json = |body: &str, id: u64, extra: &str| {
                let mut value: serde_json::Value = serde_json::from_str(body).unwrap();
                value["id"] = id.into();
                let body = value.to_string();
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n{extra}\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            };
            let request: serde_json::Value =
                serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
            let id = request["id"].as_u64().unwrap_or_default();
            let response = match (method.as_str(), request["method"].as_str()) {
                ("POST", Some("initialize")) => {
                    json(&init, id, "mcp-session-id: recorded-session\r\n")
                }
                ("POST", Some("tools/list")) => json(&list, id, ""),
                ("POST", _) => {
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                }
                ("DELETE", _) => {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                }
                _ => "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into(),
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (url, seen)
}

/// The request's method, its `mcp-session-id` header, and its body.
fn read_request(stream: &mut TcpStream) -> (String, Option<String>, Vec<u8>) {
    let mut request = Vec::new();
    let mut buf = [0; 4096];
    let head_end = loop {
        if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return (String::new(), None, Vec::new()),
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).into_owned();
    let header = |name: &str| {
        head.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    };
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    while request.len() < head_end + length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let method = head
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    (
        method,
        header("mcp-session-id"),
        request[head_end..].to_vec(),
    )
}

#[test]
fn a_url_lists_its_tools_in_one_session() {
    let (url, seen) = serve();
    let listed = tools(&Server::parse(&url).unwrap()).unwrap();
    assert_recorded(&listed);
    let seen = seen.lock().unwrap();
    // Everything after initialize carries the session it was given.
    assert_eq!(seen[0], ("POST".to_owned(), None));
    assert!(seen[1..]
        .iter()
        .all(|(_, session)| session.as_deref() == Some("recorded-session")));
    assert!(seen.iter().filter(|(m, _)| m == "POST").count() >= 3);
}

#[test]
fn unreachable_servers_say_why_in_one_line() {
    assert_eq!(
        tools(&command("gutenberg-no-such-mcp-server --stdio")).unwrap_err(),
        "gutenberg-no-such-mcp-server: command not found"
    );
    assert_eq!(
        tools(&command(
            "sh -c 'echo starting >&2; echo Error: BRAVE_API_KEY is not set >&2; exit 1'"
        ))
        .unwrap_err(),
        "the server stopped: Error: BRAVE_API_KEY is not set"
    );

    // A port nothing listens on.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let refused = tools(&Server::parse(&format!("http://127.0.0.1:{port}/mcp")).unwrap());
    assert!(
        refused.as_ref().unwrap_err().contains("refused"),
        "{refused:?}"
    );

    // A web server that isn't an MCP server.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let _ = read_request(&mut stream);
            let page = "<!DOCTYPE html>\n<pre>Cannot POST /mcp</pre>\n";
            let _ = write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{page}",
                page.len()
            );
        }
    });
    assert_eq!(
        tools(&Server::parse(&url).unwrap()).unwrap_err(),
        "HTTP 404 Not Found"
    );
}

#[test]
fn a_server_that_never_answers_times_out_and_is_ended() {
    // The server starts a process of its own and writes down its pid.
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("pid");
    let silent = Server::Command {
        command: "sh".into(),
        args: vec![
            "-c".into(),
            "sleep 30 & echo $! > \"$0\"; wait".into(),
            pidfile.display().to_string(),
        ],
    };
    let started = std::time::Instant::now();
    let err = runtime()
        .block_on(mcp::tools(&silent, Duration::from_secs(1)))
        .unwrap_err();
    assert_eq!(err.to_string(), "no answer in 1 s");
    assert!(started.elapsed() < Duration::from_secs(10));
    // Ending the server ended what it started too.
    let pid = std::fs::read_to_string(&pidfile).unwrap();
    let proc = format!("/proc/{}", pid.trim());
    let gone = (0..50).any(|_| {
        std::thread::sleep(Duration::from_millis(20));
        !std::path::Path::new(&proc).exists()
            || std::fs::read_to_string(format!("{proc}/stat"))
                .is_ok_and(|stat| stat.split_whitespace().nth(2) == Some("Z"))
    });
    assert!(gone, "{proc} is still running");
}
