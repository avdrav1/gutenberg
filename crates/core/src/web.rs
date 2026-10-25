//! What the article pane fetches: its images and, for the reader view, the
//! article's own page. One worker thread does it, so the UI never waits on
//! HTTP, the disk or on cutting a page down. With a [`Cache`], both are
//! kept on disk and read from there next time, and a page fetched goes
//! into the store's search index.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gutenberg_sources::{feed, image, reader};
use gutenberg_store::Store;
use tokio::sync::{mpsc, Semaphore};

use crate::cache::Cache;

pub use gutenberg_sources::image::Format;

/// Downloads at once.
const IN_FLIGHT: usize = 4;

/// What became of a request.
#[derive(Debug, Clone)]
pub enum Answer {
    Image {
        url: String,
        bytes: Vec<u8>,
        format: Format,
    },
    /// Unreachable, too big, or not an image.
    ImageFailed { url: String },
    /// The article on the page at `url`, as HTML.
    Page { url: String, html: String },
    /// One line on why the page has no reader view.
    PageFailed { url: String, error: String },
}

enum Request {
    Image(String),
    Page(String),
}

/// Hands requests to the worker. Cheap to clone; the worker stops when the
/// last handle is dropped.
#[derive(Debug, Clone)]
pub struct Web {
    requests: mpsc::UnboundedSender<Request>,
}

/// Where the worker keeps what it fetched: the cache's directory and cap in
/// bytes, and the store whose search index takes reader pages.
#[derive(Debug, Default)]
pub struct Keep {
    pub cache: Option<(PathBuf, u64)>,
    pub store: Option<PathBuf>,
}

/// The worker's side of [`Keep`], opened on its thread.
struct Kept {
    cache: Option<Mutex<Cache>>,
    store: Option<Mutex<Store>>,
}

impl Web {
    /// Starts the worker, keeping nothing. `on_answer` runs on its thread,
    /// once per request.
    pub fn spawn(on_answer: impl Fn(Answer) + Send + Sync + 'static) -> std::io::Result<Self> {
        Self::spawn_keeping(Keep::default(), on_answer)
    }

    /// Starts the worker, keeping what it fetches in `keep`.
    pub fn spawn_keeping(
        keep: Keep,
        on_answer: impl Fn(Answer) + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (requests, mut queue) = mpsc::unbounded_channel::<Request>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("web".into())
            .spawn(move || {
                let on_answer = Arc::new(on_answer);
                let store = keep.store.and_then(|path| {
                    Store::open(&path)
                        .map_err(|err| eprintln!("gutenberg: web: {err}"))
                        .ok()
                });
                let kept = Arc::new(Kept {
                    cache: keep
                        .cache
                        .map(|(dir, cap)| Mutex::new(Cache::open(dir, cap))),
                    store: store.map(Mutex::new),
                });
                let slots = Arc::new(Semaphore::new(IN_FLIGHT));
                runtime.block_on(async move {
                    let client = match feed::client() {
                        Ok(client) => client,
                        Err(err) => {
                            eprintln!("gutenberg: web: {err}");
                            return;
                        }
                    };
                    while let Some(request) = queue.recv().await {
                        let (client, on_answer, kept) =
                            (client.clone(), on_answer.clone(), kept.clone());
                        let Ok(slot) = slots.clone().acquire_owned().await else {
                            return;
                        };
                        tokio::spawn(async move {
                            let answer = answer(&client, &kept, request).await;
                            drop(slot);
                            on_answer(answer);
                        });
                    }
                });
            })?;
        Ok(Self { requests })
    }

    /// Asks for an image; the answer comes to `on_answer`.
    pub fn image(&self, url: String) {
        self.send(Request::Image(url));
    }

    /// Asks for the article on a page, for the reader view.
    pub fn page(&self, url: String) {
        self.send(Request::Page(url));
    }

    fn send(&self, request: Request) {
        // A closed channel means the worker is gone; there's no one to tell.
        let _ = self.requests.send(request);
    }
}

async fn answer(client: &feed::Client, kept: &Kept, request: Request) -> Answer {
    match request {
        Request::Image(url) => {
            if let Some((bytes, format)) = kept.cached(|cache| cache.image(&url)) {
                return Answer::Image { url, bytes, format };
            }
            match image::fetch(client, &url).await {
                Some((bytes, format)) => {
                    kept.save(|cache| cache.save_image(&url, &bytes, format));
                    Answer::Image { url, bytes, format }
                }
                None => Answer::ImageFailed { url },
            }
        }
        Request::Page(url) => {
            if let Some(html) = kept.cached(|cache| cache.page(&url)) {
                return Answer::Page { url, html };
            }
            page(client, kept, url).await
        }
    }
}

async fn page(client: &feed::Client, kept: &Kept, url: String) -> Answer {
    match reader::fetch(client, &url).await {
        Ok(extracted) => {
            kept.save(|cache| cache.save_page(&url, &extracted.html));
            if let Some(store) = &kept.store {
                let indexed = store
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .index_page(&url, &extracted.html);
                if let Err(err) = indexed {
                    eprintln!("gutenberg: indexing {url}: {err}");
                }
            }
            Answer::Page {
                url,
                html: extracted.html,
            }
        }
        Err(err) => Answer::PageFailed {
            url,
            error: err.to_string(),
        },
    }
}

impl Kept {
    fn cached<T>(&self, read: impl FnOnce(&Cache) -> Option<T>) -> Option<T> {
        let cache = self.cache.as_ref()?;
        read(
            &cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// A full disk or a cache directory that can't be made costs only the
    /// next fetch.
    fn save(&self, write: impl FnOnce(&mut Cache) -> std::io::Result<()>) {
        if let Some(cache) = &self.cache {
            let mut cache = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(err) = write(&mut cache) {
                eprintln!("gutenberg: cache: {err}");
            }
        }
    }
}
