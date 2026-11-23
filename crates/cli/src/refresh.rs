//! `gutenberg --refresh`: refresh every due source now and say how it
//! went. With a window open, its refresh thread does it, over the socket;
//! otherwise this process does, and then notifies as the window would
//! (S10.3). That's what the systemd timer `install.sh` offers runs.
//!
//! Two runs never refresh at once: this one takes the refresh lock the
//! window's thread also takes, and finding it held, exits. Run by the timer
//! (stdout isn't a terminal), it stays after its pop-ups until each has
//! closed or Open is clicked, at most [`STAY`], so Open has a process to
//! answer it; from a terminal it doesn't stay, and its pop-ups offer no Open.

use std::process::ExitCode;
#[cfg(target_os = "linux")]
use std::time::Duration;

use gutenberg_core::favicon::Icons;
use gutenberg_core::ipc;
use gutenberg_core::lock::{self, RefreshLock};
use gutenberg_core::sync;
use gutenberg_store::Store;

/// Longest a timer's run waits on its pop-ups: well inside the timer's 15
/// minutes, so it's gone before the next run.
#[cfg(target_os = "linux")]
const STAY: Duration = Duration::from_secs(10 * 60);

pub fn run() -> ExitCode {
    if let Some(path) = ipc::socket_path() {
        match ipc::send(&path, ipc::Command::Refresh) {
            // The window shows what failed in its sources pane.
            Ok(summary) => {
                println!("{summary}");
                return ExitCode::SUCCESS;
            }
            Err(ipc::Error::NotRunning) => {}
            Err(err) => {
                eprintln!("gutenberg: refresh: {err}");
                return ExitCode::FAILURE;
            }
        }
    }
    let turn = match lock::path().map(|path| RefreshLock::try_take(&path)) {
        Some(Ok(Some(turn))) => Some(turn),
        Some(Ok(None)) => {
            println!("Another refresh is running");
            return ExitCode::SUCCESS;
        }
        Some(Err(err)) => {
            eprintln!("gutenberg: refresh lock: {err}");
            None
        }
        None => None,
    };
    let config = crate::config();
    let (mut store, summary, now) = match refresh(&config) {
        Ok(done) => done,
        Err(message) => {
            eprintln!("gutenberg: refresh: {message}");
            return ExitCode::FAILURE;
        }
    };
    #[cfg(target_os = "linux")]
    let pop_ups = notify::deliver(&mut store, &config.notify, &summary, now);
    #[cfg(not(target_os = "linux"))]
    let _ = (&mut store, now);
    drop(turn);
    for (url, error) in &summary.failed {
        eprintln!("gutenberg: {url}: {error}");
    }
    println!("{summary}");
    #[cfg(target_os = "linux")]
    pop_ups.stay(STAY);
    ExitCode::SUCCESS
}

fn refresh(config: &gutenberg_core::config::Config) -> Result<(Store, sync::Summary, i64), String> {
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let now = sync::unix_now();
    let summary = runtime
        .block_on(sync::refresh(
            &mut store,
            now,
            &crate::limits_from(config),
            Some(&Icons::beside(&path)),
        ))
        .map_err(|e| e.to_string())?;
    Ok((store, summary, now))
}

/// Pop-ups from a refresh with no window, and staying for their Open.
#[cfg(target_os = "linux")]
mod notify {
    use std::collections::HashSet;
    use std::io::IsTerminal;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use gutenberg_core::config;
    use gutenberg_core::notify::{self, Desktop, Event, Popup, Sender};
    use gutenberg_core::sync::Summary;
    use gutenberg_store::Store;

    /// What this run sent, and where the desktop says what became of it.
    pub struct PopUps {
        shown: HashSet<u32>,
        events: mpsc::Receiver<Event>,
        // Keeps the desktop's thread, and its connection, until the stay
        // is over.
        _desktop: Option<Desktop>,
    }

    /// Shows the pop-ups `summary` and the store call for, as the window
    /// would with no focus.
    pub fn deliver(
        store: &mut Store,
        settings: &config::Notify,
        summary: &Summary,
        now: i64,
    ) -> PopUps {
        struct Lazy {
            desktop: Option<Desktop>,
            stays: bool,
            events: mpsc::Sender<Event>,
            shown: HashSet<u32>,
        }
        impl Sender for Lazy {
            fn send(&mut self, popup: &Popup) -> Option<u32> {
                let (events, stays) = (self.events.clone(), self.stays);
                let id = self
                    .desktop
                    .get_or_insert_with(|| {
                        Desktop::watch(stays, move |event| {
                            let _ = events.send(event);
                        })
                    })
                    .send(popup)?;
                self.shown.insert(id);
                Some(id)
            }
        }
        let (events, received) = mpsc::channel();
        let mut lazy = Lazy {
            desktop: None,
            stays: !std::io::stdout().is_terminal(),
            events,
            shown: HashSet::new(),
        };
        if let Err(err) = notify::deliver(store, settings, &summary.matched, now, false, &mut lazy)
        {
            eprintln!("gutenberg: notifications: {err}");
        }
        PopUps {
            shown: if lazy.stays {
                lazy.shown
            } else {
                HashSet::new()
            },
            events: received,
            _desktop: lazy.desktop,
        }
    }

    impl PopUps {
        /// Waits, at most `longest`, until every pop-up has closed or one's
        /// Open is clicked; Open starts the window on its article.
        pub fn stay(mut self, longest: Duration) {
            let until = Instant::now() + longest;
            while !self.shown.is_empty() {
                let left = until.saturating_duration_since(Instant::now());
                match self.events.recv_timeout(left) {
                    Ok(Event::Open(article)) => {
                        crate::open::launch(article);
                        return;
                    }
                    Ok(Event::Closed(id)) => {
                        self.shown.remove(&id);
                    }
                    Err(_) => return,
                }
            }
        }
    }
}
