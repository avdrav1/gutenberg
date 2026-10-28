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

/// Widest raster image handed to the window, in pixels; wider ones are
/// scaled down. The article pane is narrower than this on a 4K screen.
pub const MAX_WIDTH: u32 = 1600;
/// Tallest. With [`MAX_WIDTH`], a picture fits one 2048-pixel layer of
/// iced's texture atlas, which grows to hold the largest image shown and
/// never shrinks.
pub const MAX_HEIGHT: u32 = 2048;

/// An image ready to draw.
#[derive(Clone)]
pub enum Picture {
    /// Decoded, at most [`MAX_WIDTH`] by [`MAX_HEIGHT`].
    Rgba {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
    /// An SVG document, drawn at any size.
    Svg(Vec<u8>),
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rgba { width, height, .. } => write!(f, "Rgba({width}x{height})"),
            Self::Svg(bytes) => write!(f, "Svg({} bytes)", bytes.len()),
        }
    }
}

/// What became of a request.
#[derive(Debug, Clone)]
pub enum Answer {
    Image {
        url: String,
        picture: Picture,
    },
    /// Unreachable, too big, or not an image that decodes.
    ImageFailed {
        url: String,
    },
    /// The article on the page at `url`, as HTML.
    Page {
        url: String,
        html: String,
    },
    /// One line on why the page has no reader view.
    PageFailed {
        url: String,
        error: String,
    },
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
            let fetched = match kept.cached(|cache| cache.image(&url)) {
                Some(cached) => Some(cached),
                None => image::fetch(client, &url).await.inspect(|(bytes, format)| {
                    kept.save(|cache| cache.save_image(&url, bytes, *format));
                }),
            };
            match fetched.and_then(|(bytes, format)| picture(bytes, format)) {
                Some(picture) => Answer::Image { url, picture },
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

/// Decodes a raster image, scaled down to fit [`MAX_WIDTH`] by
/// [`MAX_HEIGHT`], so a large photo costs the window no more than one
/// atlas layer.
fn picture(bytes: Vec<u8>, format: Format) -> Option<Picture> {
    let format = match format {
        Format::Svg => return Some(Picture::Svg(bytes)),
        Format::Png => ::image::ImageFormat::Png,
        Format::Ico => ::image::ImageFormat::Ico,
        Format::Gif => ::image::ImageFormat::Gif,
        Format::Jpeg => ::image::ImageFormat::Jpeg,
        Format::Webp => ::image::ImageFormat::WebP,
    };
    let mut decoded = ::image::load_from_memory_with_format(&bytes, format).ok()?;
    drop(bytes);
    if decoded.width() > MAX_WIDTH || decoded.height() > MAX_HEIGHT {
        decoded = decoded.resize(
            MAX_WIDTH,
            MAX_HEIGHT,
            ::image::imageops::FilterType::Triangle,
        );
    }
    let rgba = decoded.into_rgba8();
    let picture = Picture::Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    };
    give_back_freed_memory();
    Some(picture)
}

/// Decoding a large photo takes tens of megabytes for a moment. glibc
/// keeps memory freed on this thread for later, which held the window at
/// 155 MB after 20 articles with photos (S7.4); returned, it stays near
/// 115 MB. Other allocators (macOS's malloc among them) return free pages
/// on their own, so this does nothing there.
fn give_back_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim has no preconditions; it only releases free pages.
    unsafe {
        libc::malloc_trim(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        ::image::RgbaImage::new(width, height)
            .write_to(&mut bytes, ::image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn size(picture: Option<Picture>) -> Option<(u32, u32)> {
        match picture? {
            Picture::Rgba {
                width,
                height,
                pixels,
            } => {
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
                Some((width, height))
            }
            Picture::Svg(_) => None,
        }
    }

    #[test]
    fn large_images_are_scaled_to_fit_one_atlas_layer() {
        assert_eq!(size(picture(png(800, 600), Format::Png)), Some((800, 600)));
        // Kept in proportion.
        assert_eq!(
            size(picture(png(4000, 1000), Format::Png)),
            Some((1600, 400))
        );
        assert_eq!(
            size(picture(png(1000, 8000), Format::Png)),
            Some((256, 2048))
        );
    }

    #[test]
    fn what_doesnt_decode_is_no_picture_and_svg_stays_svg() {
        assert!(picture(b"not a png".to_vec(), Format::Png).is_none());
        // A PNG sent as a JPEG is read as what it claims to be.
        assert!(picture(png(10, 10), Format::Jpeg).is_none());
        assert!(matches!(
            picture(b"<svg/>".to_vec(), Format::Svg),
            Some(Picture::Svg(_))
        ));
    }
}
