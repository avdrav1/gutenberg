//! What the article pane fetches: its images and, for the reader view, the
//! article's own page. One worker thread does it, so the UI never waits on
//! HTTP or on cutting a page down. Nothing goes to disk.

use std::sync::Arc;

use gutenberg_sources::{feed, image, reader};
use tokio::sync::{mpsc, Semaphore};

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

impl Web {
    /// Starts the worker. `on_answer` runs on its thread, once per request.
    pub fn spawn(on_answer: impl Fn(Answer) + Send + Sync + 'static) -> std::io::Result<Self> {
        let (requests, mut queue) = mpsc::unbounded_channel::<Request>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("web".into())
            .spawn(move || {
                let on_answer = Arc::new(on_answer);
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
                        let (client, on_answer) = (client.clone(), on_answer.clone());
                        let Ok(slot) = slots.clone().acquire_owned().await else {
                            return;
                        };
                        tokio::spawn(async move {
                            let answer = answer(&client, request).await;
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

async fn answer(client: &feed::Client, request: Request) -> Answer {
    match request {
        Request::Image(url) => match image::fetch(client, &url).await {
            Some((bytes, format)) => Answer::Image { url, bytes, format },
            None => Answer::ImageFailed { url },
        },
        Request::Page(url) => match reader::fetch(client, &url).await {
            Ok(extracted) => Answer::Page {
                url,
                html: extracted.html,
            },
            Err(err) => Answer::PageFailed {
                url,
                error: err.to_string(),
            },
        },
    }
}
