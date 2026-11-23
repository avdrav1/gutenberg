//! Pop-ups over D-Bus, to `org.freedesktop.Notifications` (on Omarchy,
//! its shell). A thread of its own holds the session-bus connection, so it
//! hears Open on a pop-up for as long as the app runs; the refresh thread
//! hands it pop-ups and waits at most [`WAIT`] for the desktop's id. A
//! pop-up that replaces another passes its id, so the desktop updates it
//! in place.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use zbus::zvariant::Value;
use zbus::{Connection, Proxy};

use super::{Popup, Sender};

/// Longest the refresh thread waits for the desktop to take a pop-up.
const WAIT: Duration = Duration::from_secs(2);

type Ask = (Popup, mpsc::Sender<Option<u32>>);

/// The desktop's notification service, reached from its own thread.
pub struct Desktop {
    asks: tokio::sync::mpsc::UnboundedSender<Ask>,
}

/// What became of a pop-up this desktop sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Open (or the pop-up itself) was clicked: show this article.
    Open(i64),
    /// The pop-up with this id went: dismissed, expired or replaced.
    Closed(u32),
}

impl Desktop {
    /// Starts the thread; `on_open` gets the article of a pop-up whose Open
    /// (or body) was clicked, on that thread.
    pub fn spawn(on_open: impl Fn(i64) + Send + 'static) -> Self {
        Self::watch(true, move |event| {
            if let Event::Open(article) = event {
                on_open(article);
            }
        })
    }

    /// As [`spawn`](Self::spawn), with every [`Event`] of the pop-ups it
    /// sends. Without `actions` a pop-up offers no Open, for a process that
    /// won't stay to answer it.
    pub fn watch(actions: bool, on_event: impl Fn(Event) + Send + 'static) -> Self {
        let (asks, asked) = tokio::sync::mpsc::unbounded_channel();
        let spawned = std::thread::Builder::new()
            .name("notify".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => return eprintln!("gutenberg: notifications: {err}"),
                };
                runtime.block_on(serve(asked, actions, on_event));
            });
        if let Err(err) = spawned {
            eprintln!("gutenberg: notifications did not start: {err}");
        }
        Self { asks }
    }
}

impl Sender for Desktop {
    fn send(&mut self, popup: &Popup) -> Option<u32> {
        let (reply, answer) = mpsc::channel();
        self.asks.send((popup.clone(), reply)).ok()?;
        answer.recv_timeout(WAIT).ok().flatten()
    }
}

async fn serve(
    mut asked: tokio::sync::mpsc::UnboundedReceiver<Ask>,
    offer_actions: bool,
    on_event: impl Fn(Event),
) {
    let proxy = match connect().await {
        Ok(proxy) => proxy,
        Err(err) => {
            eprintln!("gutenberg: no desktop notifications: {err}");
            while let Some((_, reply)) = asked.recv().await {
                let _ = reply.send(None);
            }
            return;
        }
    };
    let caps: Vec<String> = proxy.call("GetCapabilities", &()).await.unwrap_or_default();
    let markup = caps.iter().any(|c| c == "body-markup");
    let actions = offer_actions && caps.iter().any(|c| c == "actions");
    let opens: Arc<Mutex<HashMap<u32, i64>>> = Arc::default();
    // Ours: the desktop says when any pop-up closes.
    let mut sent: HashSet<u32> = HashSet::new();
    let mut invoked = match proxy.receive_signal("ActionInvoked").await {
        Ok(stream) => Some(stream),
        Err(err) => {
            eprintln!("gutenberg: notifications can't be opened: {err}");
            None
        }
    };
    let mut closed = proxy.receive_signal("NotificationClosed").await.ok();
    loop {
        tokio::select! {
            ask = asked.recv() => {
                let Some((popup, reply)) = ask else { return };
                let id = notify(&proxy, &popup, markup, actions).await;
                if let Err(err) = &id {
                    eprintln!("gutenberg: notifying: {err}");
                }
                let id = id.ok();
                if let Some(id) = id {
                    sent.insert(id);
                    // A replaced pop-up is gone, whatever id the desktop
                    // answered with.
                    if let Some(replaced) = popup.replaces.filter(|r| *r != id) {
                        if sent.remove(&replaced) {
                            on_event(Event::Closed(replaced));
                        }
                    }
                }
                if let (Some(id), Some(article), Ok(mut opens)) = (id, popup.article(), opens.lock()) {
                    opens.insert(id, article);
                }
                let _ = reply.send(id);
            }
            Some(signal) = async {
                match invoked.as_mut() {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Ok((id, key)) = signal.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                let article = opens.lock().ok().and_then(|opens| opens.get(&id).copied());
                if let (Some(article), "default" | "open") = (article, key.as_str()) {
                    on_event(Event::Open(article));
                }
            }
            Some(signal) = async {
                match closed.as_mut() {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Ok((id, _reason)) = signal.body().deserialize::<(u32, u32)>() else {
                    continue;
                };
                if sent.remove(&id) {
                    on_event(Event::Closed(id));
                }
            }
        }
    }
}

async fn connect() -> zbus::Result<Proxy<'static>> {
    let connection = Connection::session().await?;
    Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
}

async fn notify(
    proxy: &Proxy<'_>,
    popup: &Popup,
    markup: bool,
    actions: bool,
) -> zbus::Result<u32> {
    let body = popup.body();
    let body = if markup { escape(&body) } else { body };
    // "default" is a click on the pop-up itself; Open is the button.
    let buttons: Vec<&str> = if actions && popup.article().is_some() {
        vec!["default", "Open", "open", "Open"]
    } else {
        vec![]
    };
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    hints.insert("desktop-entry", Value::from("gutenberg"));
    proxy
        .call(
            "Notify",
            &(
                "Gutenberg",
                popup.replaces.unwrap_or(0),
                "gutenberg",
                popup.summary(),
                body,
                buttons,
                hints,
                -1i32,
            ),
        )
        .await
}

/// What body markup would read as a tag or an entity.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn markup_is_escaped() {
        assert_eq!(super::escape("a < b & c"), "a &lt; b &amp; c");
    }
}
