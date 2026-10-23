//! Article images, fetched on a worker thread so the UI never waits on
//! HTTP. Nothing goes to disk: the UI keeps what the open article shows.

use std::sync::Arc;

use gutenberg_sources::{feed, image};
use tokio::sync::{mpsc, Semaphore};

pub use gutenberg_sources::image::Format;

/// Downloads at once.
const IN_FLIGHT: usize = 4;

/// What became of a requested URL.
#[derive(Debug)]
pub enum Loaded {
    Image {
        url: String,
        bytes: Vec<u8>,
        format: Format,
    },
    /// Unreachable, too big, or not an image.
    Failed { url: String },
}

/// Hands URLs to the worker. Cheap to clone; the worker stops when the
/// last handle is dropped.
#[derive(Debug, Clone)]
pub struct Images {
    requests: mpsc::UnboundedSender<String>,
}

impl Images {
    /// Starts the worker. `on_load` runs on its thread, once per request.
    pub fn spawn(on_load: impl Fn(Loaded) + Send + Sync + 'static) -> std::io::Result<Self> {
        let (requests, mut queue) = mpsc::unbounded_channel::<String>();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        std::thread::Builder::new()
            .name("images".into())
            .spawn(move || {
                let on_load = Arc::new(on_load);
                let slots = Arc::new(Semaphore::new(IN_FLIGHT));
                runtime.block_on(async move {
                    let client = match feed::client() {
                        Ok(client) => client,
                        Err(err) => {
                            eprintln!("gutenberg: images: {err}");
                            return;
                        }
                    };
                    while let Some(url) = queue.recv().await {
                        let (client, on_load) = (client.clone(), on_load.clone());
                        let Ok(slot) = slots.clone().acquire_owned().await else {
                            return;
                        };
                        tokio::spawn(async move {
                            let loaded = match image::fetch(&client, &url).await {
                                Some((bytes, format)) => Loaded::Image { url, bytes, format },
                                None => Loaded::Failed { url },
                            };
                            drop(slot);
                            on_load(loaded);
                        });
                    }
                });
            })?;
        Ok(Self { requests })
    }

    /// Asks for `url`; the answer comes to `on_load`.
    pub fn request(&self, url: String) {
        // A closed channel means the worker is gone; there's no one to tell.
        let _ = self.requests.send(url);
    }
}
