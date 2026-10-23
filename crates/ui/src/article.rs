//! A feed's HTML summary as blocks the article pane can draw: headings,
//! paragraphs of styled spans, lists, code, quotes, images and rules.
//! Parsing is pure, so it's tested without a window. Anything that isn't
//! text or one of these (scripts, styles, frames, forms) is dropped.

use std::collections::HashMap;

use ego_tree::NodeRef;
use gutenberg_theme::{Rgb, Tokens};
use iced::widget::text::{Span as TextSpan, Wrapping};
use iced::widget::{container, image, rich_text, row, span, svg, text, Column, Space};
use iced::{border, font, Element, Font, Length};
use scraper::{Html, Node};
use url::Url;

use crate::{color, Message};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    List {
        ordered: bool,
        /// The first item's number.
        start: u32,
        items: Vec<Vec<Block>>,
    },
    /// Preformatted text, whitespace kept.
    Code(String),
    Quote(Vec<Block>),
    /// An absolute `http(s)` URL; `alt` may be empty.
    Image {
        src: String,
        alt: String,
    },
    Rule,
}

/// A run of text with one style and at most one link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
    /// Absolute `http(s)` or `mailto` URL.
    pub link: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// Parses `html`; relative URLs resolve against `base`, the article's own
/// address when it has one.
pub fn parse(html: &str, base: Option<&str>) -> Vec<Block> {
    let base = base.and_then(|b| Url::parse(b).ok());
    let document = Html::parse_fragment(html);
    let root = document.root_element();
    blocks(*root, &Context::new(base.as_ref()))
}

/// The inline state a node inherits from its ancestors.
#[derive(Clone)]
struct Context<'a> {
    base: Option<&'a Url>,
    style: Style,
    link: Option<String>,
}

impl<'a> Context<'a> {
    fn new(base: Option<&'a Url>) -> Self {
        Self {
            base,
            style: Style::default(),
            link: None,
        }
    }

    fn resolve(&self, href: &str, schemes: &[&str]) -> Option<String> {
        let href = href.trim();
        let url = match self.base {
            Some(base) => base.join(href).ok()?,
            None => Url::parse(href).ok()?,
        };
        schemes.contains(&url.scheme()).then(|| url.into())
    }
}

/// Elements whose content is never shown.
const DROPPED: &[&str] = &[
    "script", "style", "iframe", "noscript", "template", "form", "button", "input", "select",
    "textarea", "object", "embed", "svg", "math", "head", "title", "meta", "link", "video",
    "audio", "canvas",
];

/// Elements that end the paragraph before them and start one after.
const BREAKS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "figure",
    "figcaption",
    "details",
    "summary",
    "center",
    "address",
    "dl",
    "dt",
    "dd",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "caption",
    "picture",
];

/// The blocks `node`'s children make.
fn blocks(node: NodeRef<'_, Node>, cx: &Context<'_>) -> Vec<Block> {
    let mut flow = Flow::default();
    for child in node.children() {
        walk(child, cx, &mut flow);
    }
    flow.finish()
}

fn walk(node: NodeRef<'_, Node>, cx: &Context<'_>, flow: &mut Flow) {
    let element = match node.value() {
        Node::Text(text) => return flow.text(text, cx),
        Node::Element(element) => element,
        _ => return,
    };
    let name = element.name();
    match name {
        _ if DROPPED.contains(&name) => {}
        "br" => flow.line_break(cx),
        "hr" => flow.block(Block::Rule),
        "img" => {
            if let Some(image) = image_block(element, cx) {
                flow.block(image);
            }
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = name[1..].parse().unwrap_or(1);
            let inner = Context {
                style: Style {
                    bold: true,
                    ..cx.style
                },
                ..cx.clone()
            };
            let mut spans = Vec::new();
            let mut rest = Vec::new();
            for block in blocks(node, &inner) {
                match block {
                    Block::Paragraph(more) if rest.is_empty() => {
                        if !spans.is_empty() {
                            push_span(&mut spans, " ", inner.style, None);
                        }
                        for span in more {
                            push_span(&mut spans, &span.text, span.style, span.link);
                        }
                    }
                    other => rest.push(other),
                }
            }
            if !spans.is_empty() {
                flow.block(Block::Heading { level, spans });
            }
            for block in rest {
                flow.block(block);
            }
        }
        "ul" | "ol" => {
            let items: Vec<Vec<Block>> = node
                .children()
                .filter_map(|child| match child.value() {
                    Node::Element(e) if e.name() == "li" => Some(blocks(child, cx)),
                    Node::Element(_) => Some(blocks_of(child, cx)),
                    _ => None,
                })
                .filter(|item| !item.is_empty())
                .collect();
            if !items.is_empty() {
                let start = element.attr("start").and_then(|s| s.trim().parse().ok());
                flow.block(Block::List {
                    ordered: name == "ol",
                    start: start.unwrap_or(1),
                    items,
                });
            }
        }
        "pre" => {
            let code = preformatted(node);
            if !code.trim().is_empty() {
                flow.block(Block::Code(code));
            }
        }
        "blockquote" => {
            let quoted = blocks(node, cx);
            if !quoted.is_empty() {
                flow.block(Block::Quote(quoted));
            }
        }
        "li" => {
            // An item outside a list: a paragraph of its own.
            flow.end_paragraph();
            walk_children(node, cx, flow);
            flow.end_paragraph();
        }
        "td" | "th" => {
            walk_children(node, cx, flow);
            flow.text(" ", cx);
        }
        _ if BREAKS.contains(&name) => {
            flow.end_paragraph();
            walk_children(node, cx, flow);
            flow.end_paragraph();
        }
        _ => {
            let mut inner = cx.clone();
            match name {
                "b" | "strong" => inner.style.bold = true,
                "i" | "em" | "cite" | "dfn" | "var" => inner.style.italic = true,
                "code" | "kbd" | "samp" | "tt" => inner.style.code = true,
                "a" => {
                    if let Some(href) = element.attr("href") {
                        inner.link = cx.resolve(href, &["http", "https", "mailto"]);
                    }
                }
                _ => {}
            }
            walk_children(node, &inner, flow);
        }
    }
}

fn walk_children(node: NodeRef<'_, Node>, cx: &Context<'_>, flow: &mut Flow) {
    for child in node.children() {
        walk(child, cx, flow);
    }
}

/// The blocks a node makes when it's itself the content, not just its
/// children: a stray element directly inside a list.
fn blocks_of(node: NodeRef<'_, Node>, cx: &Context<'_>) -> Vec<Block> {
    let mut flow = Flow::default();
    walk(node, cx, &mut flow);
    flow.finish()
}

/// An `<img>` worth showing: the real source of a lazy-loaded image, and
/// no 1×1 tracking pixels.
fn image_block(element: &scraper::node::Element, cx: &Context<'_>) -> Option<Block> {
    let tiny = |attr| {
        element
            .attr(attr)
            .and_then(|v| v.trim().trim_end_matches("px").parse::<u32>().ok())
            .is_some_and(|n| n <= 1)
    };
    if tiny("width") || tiny("height") {
        return None;
    }
    let src = ["src", "data-src", "data-lazy-src"]
        .into_iter()
        .filter_map(|attr| element.attr(attr))
        .find_map(|src| cx.resolve(src, &["http", "https"]))?;
    let alt = element.attr("alt").unwrap_or_default();
    Some(Block::Image {
        src,
        alt: collapse(alt).trim().to_owned(),
    })
}

/// A `<pre>`'s text as written, without the newline that may follow the tag.
fn preformatted(node: NodeRef<'_, Node>) -> String {
    let mut text = String::new();
    for descendant in node.descendants() {
        match descendant.value() {
            Node::Text(t) => text.push_str(t),
            Node::Element(e) if e.name() == "br" => text.push('\n'),
            _ => {}
        }
    }
    let text = text.strip_prefix('\n').unwrap_or(&text);
    text.trim_end().to_owned()
}

/// Whitespace runs as single spaces, the way a browser shows text.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

/// Appends text, joining it to the last span when the style and link match.
fn push_span(spans: &mut Vec<Span>, text: &str, style: Style, link: Option<String>) {
    match spans.last_mut() {
        Some(last) if last.style == style && last.link == link => last.text.push_str(text),
        _ => spans.push(Span {
            text: text.to_owned(),
            style,
            link,
        }),
    }
}

/// Blocks so far and the paragraph being filled.
#[derive(Default)]
struct Flow {
    blocks: Vec<Block>,
    spans: Vec<Span>,
}

impl Flow {
    fn text(&mut self, text: &str, cx: &Context<'_>) {
        let mut text = collapse(text);
        // One space between words, none at the start of a line.
        let at_line_start = self
            .spans
            .last()
            .is_none_or(|s| s.text.ends_with([' ', '\n']));
        if at_line_start {
            text = text.trim_start().to_owned();
        }
        if !text.is_empty() {
            push_span(&mut self.spans, &text, cx.style, cx.link.clone());
        }
    }

    fn line_break(&mut self, cx: &Context<'_>) {
        if let Some(last) = self.spans.last_mut() {
            let kept = last.text.trim_end().len();
            last.text.truncate(kept);
        }
        if !self.spans.is_empty() {
            push_span(&mut self.spans, "\n", cx.style, cx.link.clone());
        }
    }

    fn block(&mut self, block: Block) {
        self.end_paragraph();
        self.blocks.push(block);
    }

    fn end_paragraph(&mut self) {
        let mut spans = std::mem::take(&mut self.spans);
        while let Some(last) = spans.last_mut() {
            let kept = last.text.trim_end().len();
            last.text.truncate(kept);
            if !last.text.is_empty() {
                break;
            }
            spans.pop();
        }
        if !spans.is_empty() {
            self.blocks.push(Block::Paragraph(spans));
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.end_paragraph();
        self.blocks
    }
}

/// An article image, as far as it has got.
#[derive(Debug, Clone)]
pub enum Picture {
    Loading,
    /// Decoded and on the GPU, so it lays out at its real size at once.
    Raster(image::Allocation),
    Vector(svg::Handle),
    Failed,
}

/// Every image URL in `blocks`, nested ones included, in order.
pub fn image_urls(blocks: &[Block]) -> Vec<&str> {
    let mut urls = Vec::new();
    for block in blocks {
        match block {
            Block::Image { src, .. } => urls.push(src.as_str()),
            Block::List { items, .. } => items.iter().for_each(|i| urls.extend(image_urls(i))),
            Block::Quote(inner) => urls.extend(image_urls(inner)),
            _ => {}
        }
    }
    urls
}

/// Body text size; headings scale up from it.
const BODY: f32 = 15.0;

/// The blocks as one column that wraps to the pane's width: long words
/// break, code wraps, and images scale down to fit.
pub fn view<'a>(
    blocks: &'a [Block],
    t: &Tokens,
    pictures: &'a HashMap<String, Picture>,
) -> Element<'a, Message> {
    Column::with_children(blocks.iter().map(|b| block_view(b, t, pictures)))
        .spacing(12)
        .width(Length::Fill)
        .into()
}

fn block_view<'a>(
    block: &'a Block,
    t: &Tokens,
    pictures: &'a HashMap<String, Picture>,
) -> Element<'a, Message> {
    match block {
        Block::Paragraph(spans) => spans_view(spans, BODY, t),
        Block::Heading { level, spans } => {
            let size = match level {
                1 => 21.0,
                2 => 19.0,
                3 => 17.0,
                _ => BODY,
            };
            spans_view(spans, size, t)
        }
        Block::List {
            ordered,
            start,
            items,
        } => Column::with_children(items.iter().enumerate().map(|(i, item)| {
            let marker = if *ordered {
                format!("{}.", *start as usize + i)
            } else {
                "•".to_owned()
            };
            row![
                text(marker)
                    .size(BODY)
                    .color(color(t.muted))
                    .width(Length::Fixed(28.0)),
                view(item, t, pictures),
            ]
            .into()
        }))
        .spacing(6)
        .into(),
        Block::Code(code) => {
            let background = t.background;
            container(
                text(code)
                    .size(13)
                    .font(Font::MONOSPACE)
                    .color(color(t.foreground))
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .padding(12)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(color(background).into()),
                border: border::rounded(4),
                ..Default::default()
            })
            .into()
        }
        Block::Quote(inner) => row![
            bar(t.border, Length::Fixed(3.0), Length::Fill),
            view(inner, t, pictures)
        ]
        .spacing(12)
        .into(),
        Block::Image { src, alt } => match pictures.get(src) {
            Some(Picture::Raster(allocation)) => image(allocation.handle().clone()).into(),
            Some(Picture::Vector(handle)) => svg(handle.clone()).width(Length::Fill).into(),
            Some(Picture::Loading) | None => placeholder("Loading image", alt, t),
            Some(Picture::Failed) => placeholder("Image unavailable", alt, t),
        },
        Block::Rule => bar(t.border, Length::Fill, Length::Fixed(1.0)),
    }
}

/// Styled text; links are underlined in the accent color and open the
/// browser when clicked.
fn spans_view<'a>(spans: &'a [Span], size: f32, t: &Tokens) -> Element<'a, Message> {
    let pieces: Vec<TextSpan<'a, String, Font>> = spans
        .iter()
        .map(|s| {
            let family = if s.style.code {
                Font::MONOSPACE
            } else {
                Font::DEFAULT
            };
            let font = Font {
                weight: if s.style.bold {
                    font::Weight::Bold
                } else {
                    font::Weight::Normal
                },
                style: if s.style.italic {
                    font::Style::Italic
                } else {
                    font::Style::Normal
                },
                ..family
            };
            let piece = span(s.text.as_str()).font(font);
            match &s.link {
                Some(link) => piece
                    .color(color(t.accent))
                    .underline(true)
                    .link(link.clone()),
                None => piece,
            }
        })
        .collect();
    rich_text(pieces)
        .size(size)
        .color(color(t.foreground))
        .wrapping(Wrapping::WordOrGlyph)
        .on_link_click(Message::OpenLink)
        .into()
}

/// Stands in for an image that isn't drawn: why, and its alt text.
fn placeholder<'a>(state: &str, alt: &str, t: &Tokens) -> Element<'a, Message> {
    let label = if alt.is_empty() {
        state.to_owned()
    } else {
        format!("{state}: {alt}")
    };
    text(label)
        .size(13)
        .color(color(t.muted))
        .wrapping(Wrapping::WordOrGlyph)
        .into()
}

/// A solid bar: a quote's left edge, or a rule.
fn bar<'a>(c: Rgb, width: Length, height: Length) -> Element<'a, Message> {
    container(Space::new())
        .width(width)
        .height(height)
        .style(move |_| container::Style {
            background: Some(color(c).into()),
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!(
            "{}/../../tests/fixtures/summaries/{name}.html",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// One letter per block, lists with their item count: `P L3 H2 C`.
    fn shape(blocks: &[Block]) -> String {
        let one = |b: &Block| match b {
            Block::Heading { level, .. } => format!("H{level}"),
            Block::Paragraph(_) => "P".into(),
            Block::List { items, .. } => format!("L{}", items.len()),
            Block::Code(_) => "C".into(),
            Block::Quote(_) => "Q".into(),
            Block::Image { .. } => "I".into(),
            Block::Rule => "R".into(),
        };
        blocks.iter().map(one).collect::<Vec<_>>().join(" ")
    }

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn plain(text: &str) -> Span {
        Span {
            text: text.into(),
            style: Style::default(),
            link: None,
        }
    }

    fn code(text: &str) -> Span {
        Span {
            style: Style {
                code: true,
                ..Style::default()
            },
            ..plain(text)
        }
    }

    #[test]
    fn arch_news_keeps_its_list_of_code_and_its_links() {
        let blocks = parse(&fixture("archlinux-news"), None);
        assert_eq!(shape(&blocks), "P P L9 P");
        let Block::Paragraph(first) = &blocks[0] else {
            panic!()
        };
        assert_eq!(
            first[1],
            Span {
                link: Some("https://vinyl-cache.org/organization/on_vinyl_cache_and_varnish_cache.html#org-vinyl-varnish".into()),
                ..plain("renamed itself to Vinyl Cache")
            }
        );
        let Block::List { ordered, items, .. } = &blocks[2] else {
            panic!()
        };
        assert!(!ordered);
        assert_eq!(
            items[0],
            [Block::Paragraph(vec![
                plain("rename "),
                code("/etc/varnish"),
                plain(" to "),
                code("/etc/vinyl-cache"),
            ])]
        );
    }

    #[test]
    fn release_notes_keep_headings_and_a_code_block() {
        let blocks = parse(&fixture("omarchy-release"), None);
        assert_eq!(shape(&blocks), "P P P L2 H2 L6 P H2 P C");
        let Block::Heading { spans, .. } = &blocks[4] else {
            panic!()
        };
        assert_eq!(text(spans), "Fixes");
        assert!(spans.iter().all(|s| s.style.bold));
        assert_eq!(
            blocks[9],
            Block::Code(
                "mv ~/.local/bin/openclaw ~/.local/bin/openclaw.mine && systemctl --user \
                 reset-failed openclaw-gateway.service && omarchy-install-openclaw-cli --now \
                 && systemctl --user start openclaw-gateway.service"
                    .into()
            )
        );
    }

    #[test]
    fn a_long_article_keeps_images_quote_and_line_breaks() {
        let blocks = parse(&fixture("propublica"), None);
        let count = |kind: &str| shape(&blocks).split(' ').filter(|k| *k == kind).count();
        // The raw file has 6 <img>, 4 <h3>, 1 <blockquote> and 2 <hr>.
        assert_eq!(
            (count("I"), count("H3"), count("Q"), count("R")),
            (6, 4, 1, 2)
        );
        assert_eq!(
            blocks[0],
            Block::Image {
                src: "https://www.propublica.org/wp-content/uploads/2026/09/hiresArtboard-2-copy.jpg?w=1149".into(),
                alt: "A collage of multiple overlapping legal documents partially obscuring a man’s face.".into(),
            }
        );
        let quote = blocks
            .iter()
            .find(|b| matches!(b, Block::Quote(_)))
            .unwrap();
        let Block::Quote(inner) = quote else { panic!() };
        assert_eq!(shape(inner), "P P");
        let letter = blocks
            .iter()
            .find_map(|b| match b {
                Block::Paragraph(s) if text(s).starts_with("Dear Judge Pagano:") => Some(text(s)),
                _ => None,
            })
            .unwrap();
        assert!(
            letter.starts_with("Dear Judge Pagano:\nI am writing"),
            "{letter}"
        );
    }

    #[test]
    fn scripts_are_dropped_and_every_link_is_absolute() {
        let blocks = parse(&fixture("carbonbrief"), None);
        let mut spans = Vec::new();
        let mut images = 0;
        fn collect<'a>(blocks: &'a [Block], spans: &mut Vec<&'a Span>, images: &mut usize) {
            for block in blocks {
                match block {
                    Block::Heading { spans: s, .. } | Block::Paragraph(s) => spans.extend(s),
                    Block::List { items, .. } => {
                        items.iter().for_each(|i| collect(i, spans, images));
                    }
                    Block::Quote(inner) => collect(inner, spans, images),
                    Block::Image { .. } => *images += 1,
                    Block::Code(_) | Block::Rule => {}
                }
            }
        }
        collect(&blocks, &mut spans, &mut images);
        assert!(!spans.iter().any(|s| s.text.contains("gform")));
        // 8 <img> in the file; one sits in a newsletter <form>, dropped with
        // the form's own text.
        assert!(!spans
            .iter()
            .any(|s| s.text.contains("for validation purposes")));
        assert_eq!(images, 7);
        assert!(spans
            .iter()
            .filter_map(|s| s.link.as_deref())
            .all(|l| l.starts_with("https://")
                || l.starts_with("http://")
                || l.starts_with("mailto:")));
    }

    #[test]
    fn lists_nest_and_count_from_their_start() {
        let html = "<ol start=3><li>one<ul><li>a</li><li>b</li></ul></li><li>two</li></ol>";
        assert_eq!(
            parse(html, None),
            [Block::List {
                ordered: true,
                start: 3,
                items: vec![
                    vec![
                        Block::Paragraph(vec![plain("one")]),
                        Block::List {
                            ordered: false,
                            start: 1,
                            items: vec![
                                vec![Block::Paragraph(vec![plain("a")])],
                                vec![Block::Paragraph(vec![plain("b")])],
                            ],
                        },
                    ],
                    vec![Block::Paragraph(vec![plain("two")])],
                ],
            }]
        );
    }

    #[test]
    fn pre_keeps_its_whitespace() {
        let html = "<pre>\nfn main() {\n    println!(\"hi\");\n}\n</pre>";
        assert_eq!(
            parse(html, None),
            [Block::Code("fn main() {\n    println!(\"hi\");\n}".into())]
        );
    }

    #[test]
    fn urls_resolve_against_the_article_and_unsafe_ones_go() {
        let html = r#"<p><a href="/about">About</a> <a href="javascript:alert(1)">js</a>
            <a href="mailto:x@example.com">mail</a></p>
            <img src="data:image/gif;base64,R0lGOD" data-src="pic.jpg" alt=" A  pic ">"#;
        let blocks = parse(html, Some("https://site.example/news/story"));
        let link = |t: &str, l: &str| Span {
            link: Some(l.into()),
            ..plain(t)
        };
        assert_eq!(
            blocks,
            [
                Block::Paragraph(vec![
                    link("About", "https://site.example/about"),
                    plain(" js "),
                    link("mail", "mailto:x@example.com"),
                ]),
                Block::Image {
                    src: "https://site.example/news/pic.jpg".into(),
                    alt: "A pic".into()
                },
            ]
        );
    }

    #[test]
    fn an_image_splits_its_paragraph_and_pixels_vanish() {
        let html = r#"<p>Before <a href="https://x.example/"><img src="https://x.example/a.png"></a> after
            <img src="https://t.example/p.gif" width="1" height="1"></p>"#;
        assert_eq!(
            parse(html, None),
            [
                Block::Paragraph(vec![plain("Before")]),
                Block::Image {
                    src: "https://x.example/a.png".into(),
                    alt: String::new()
                },
                Block::Paragraph(vec![plain("after")]),
            ]
        );
    }

    #[test]
    fn bare_text_and_breaks_make_paragraphs() {
        let html = "Just text, <b>bold</b><br>  next line<div>a block</div>tail";
        let bold = Span {
            style: Style {
                bold: true,
                ..Style::default()
            },
            ..plain("bold")
        };
        assert_eq!(
            parse(html, None),
            [
                Block::Paragraph(vec![plain("Just text, "), bold, plain("\nnext line")]),
                Block::Paragraph(vec![plain("a block")]),
                Block::Paragraph(vec![plain("tail")]),
            ]
        );
    }
}
