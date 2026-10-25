//! S6.4: a server that needs a key gets it from the keyring, or from the
//! environment first, and the store keeps only the key's name. Keys live in
//! memory here (`use_memory_keyring`), never in the machine's keyring.

use std::io::{Read, Write};
use std::net::TcpListener;

use gutenberg_core::mcp::{self, Mcp, Server};
use gutenberg_core::secrets;
use gutenberg_core::sync::{missing_key, refresh, Limits};
use gutenberg_store::Store;

const REPLAY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/mcp/replay.sh"
);

const NAME: &str = "GUTENBERG_S64_TEST_KEY";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn error(store: &Store, id: i64) -> Option<String> {
    store.fetch_state(id).unwrap().error
}

#[test]
fn a_key_comes_from_the_keyring_or_the_environment_and_is_never_stored() {
    secrets::use_memory_keyring();
    let rt = runtime();
    let mut store = Store::open_in_memory().unwrap();
    // The recorded server, played only when it gets the key written in a
    // file, so the key isn't in the command the store keeps.
    let dir = tempfile::tempdir().unwrap();
    let expected = dir.path().join("key");
    std::fs::write(&expected, "s64-secret-value").unwrap();
    let guarded = format!(
        "sh -c '[ \"${NAME}\" = \"$(cat \"$0\")\" ] || {{ echo wrong key: ${NAME} >&2; exit 1; }}; exec sh {REPLAY}' {}",
        expected.display()
    );
    let mut settings = Mcp::new(Server::parse(&guarded).unwrap());
    settings.secret = Some(NAME.into());
    let id = mcp::add(&store, &settings, "Needs a key", None)
        .unwrap()
        .unwrap()
        .id;
    let refreshed = |store: &mut Store| {
        store.recheck(id).unwrap();
        rt.block_on(refresh(store, 1_000, &Limits::default(), None))
            .unwrap();
    };

    // Nowhere yet: the source pane says how to give it.
    refreshed(&mut store);
    assert_eq!(error(&store, id), Some(missing_key(NAME)));

    // From the keyring.
    rt.block_on(secrets::save(NAME, "  s64-secret-value\n"))
        .unwrap();
    refreshed(&mut store);
    assert_eq!(error(&store, id), None);
    let source = store.sources().unwrap().remove(0);
    assert_eq!(mcp::tools(&source).len(), 4);
    assert!(source.config.contains(NAME));
    assert!(!source.config.contains("s64-secret-value"));

    // The environment wins over the keyring.
    // This test is alone in its binary, so no other thread reads the
    // environment meanwhile.
    std::env::set_var(NAME, "k2");
    refreshed(&mut store);
    assert_eq!(
        error(&store, id).as_deref(),
        Some("the server stopped: wrong key: k2")
    );
    std::env::remove_var(NAME);

    // Names and keys that can't be saved.
    assert!(rt.block_on(secrets::save("NOT-A-NAME", "k")).is_err());
    assert!(rt.block_on(secrets::save(NAME, "  ")).is_err());

    // A web server that turns the key down.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut buf = [0; 8192];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let mut web = Mcp::new(Server::parse(&url).unwrap());
    web.secret = Some(NAME.into());
    let web = mcp::add(&store, &web, "Web", None).unwrap().unwrap().id;
    rt.block_on(refresh(&mut store, 1_000, &Limits::default(), None))
        .unwrap();
    let said = error(&store, web).unwrap();
    assert!(
        said.starts_with(&format!("{NAME} was turned down (")),
        "{said}"
    );
}
