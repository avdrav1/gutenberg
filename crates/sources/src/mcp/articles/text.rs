//! Results written as text for a reader rather than as data. Each shape
//! becomes JSON-like objects that `item` reads the way it reads a list:
//!
//! - **Labelled records:** `Title: …`, `URL: …`, `Published: …` lines, one
//!   record per title, as Exa's `web_search_exa` writes them. A label with
//!   nothing after it, like `Highlights:`, takes the lines below it.
//! - **Markdown tables:** a header row names each column; a cell of
//!   `key=value, key=value` pairs, as in Hugging Face's `hf_fs`, gives each
//!   pair its own field.
//! - **Markdown links:** each line with a `[title](https://…)` link starts
//!   an article; the lines under it, up to the next link or heading, are its
//!   summary. Two links at least, so one link in a paragraph isn't a list.

use serde_json::{Map, Value};

use super::{item, AUTHOR, DATE, SUMMARY, TITLE, URL};
use crate::feed::Entry;

/// The articles in labelled records.
pub(super) fn labelled(text: &str) -> Vec<Entry> {
    let known = |label: &str| {
        [TITLE, URL, SUMMARY, AUTHOR, DATE]
            .iter()
            .any(|keys| keys.iter().any(|k| k.eq_ignore_ascii_case(label)))
    };
    let mut records: Vec<Map<String, Value>> = Vec::new();
    // The label taking the lines below it, if any.
    let mut open: Option<String> = None;
    for line in text.lines() {
        let label = line
            .split_once(':')
            .filter(|(label, _)| known(label.trim()) && !label.starts_with(' '));
        if let Some((label, value)) = label {
            let key = label.trim().to_lowercase();
            let value = value.trim();
            // A title starts the next record.
            let starts = TITLE.contains(&key.as_str());
            if starts || records.is_empty() {
                records.push(Map::new());
            }
            let record = records.last_mut().expect("pushed above");
            // A label the record already has is part of the text above it.
            if record.contains_key(&key) {
                if let Some(open) = &open {
                    append(record, open, line);
                }
                continue;
            }
            record.insert(key.clone(), value.into());
            open = value.is_empty().then_some(key);
        } else if line.trim() == "---" {
            open = None;
        } else if let (Some(record), Some(open)) = (records.last_mut(), &open) {
            append(record, open, line);
        }
    }
    records
        .into_iter()
        .filter_map(|r| item(&Value::Object(r)))
        .collect()
}

fn append(record: &mut Map<String, Value>, key: &str, line: &str) {
    if let Some(Value::String(text)) = record.get_mut(key) {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim_end());
    }
}

/// The articles in the rows of Markdown tables.
pub(super) fn tables(text: &str) -> Vec<Entry> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let separator = cells(lines[i + 1]).is_some_and(|c| {
            c.iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
        });
        let Some(header) = cells(lines[i]).filter(|_| separator) else {
            i += 1;
            continue;
        };
        i += 2;
        while let Some(row) = lines.get(i).and_then(|l| cells(l)) {
            found.extend(item(&Value::Object(row_fields(&header, &row))));
            i += 1;
        }
    }
    found
}

/// A table row's cells, or `None` for a line that isn't one. `\|` is a
/// bar inside a cell.
fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    let mut cells = vec![String::new()];
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cells.last_mut().expect("one cell").push('|');
                chars.next();
            }
            '|' => cells.push(String::new()),
            c => cells.last_mut().expect("one cell").push(c),
        }
    }
    Some(cells.iter().map(|c| c.trim().to_owned()).collect())
}

/// A row as fields named by the header. A cell holding a link gives its
/// text to its column and its URL to `url`, unless a column has one.
fn row_fields(header: &[String], row: &[String]) -> Map<String, Value> {
    let mut fields = Map::new();
    let mut linked = None;
    for (name, cell) in header.iter().zip(row) {
        let name = name.to_lowercase();
        if let Some(pairs) = pairs(cell) {
            for (key, value) in pairs {
                fields.entry(key).or_insert(value.into());
            }
            continue;
        }
        let value = match link(cell) {
            Some((text, url, _)) => {
                linked.get_or_insert(url);
                text
            }
            None => cell.trim_matches('`').to_owned(),
        };
        fields.insert(name, value.into());
    }
    if let Some(url) = linked {
        fields.entry("url").or_insert(url.into());
    }
    fields
}

/// `key=value, key=value` as pairs; `None` for a cell that isn't. A piece
/// with no `key=` belongs to the value before it, which had a comma in it.
fn pairs(cell: &str) -> Option<Vec<(String, String)>> {
    let key = |piece: &str| {
        let (key, value) = piece.split_once('=')?;
        let words = key.chars().all(|c| c.is_ascii_alphabetic() || c == ' ');
        (words && !key.is_empty() && key.len() <= 30)
            .then(|| (key.trim().to_lowercase(), value.to_owned()))
    };
    let mut pairs: Vec<(String, String)> = Vec::new();
    for piece in cell.split(", ") {
        match key(piece) {
            Some(pair) => pairs.push(pair),
            None => pairs.last_mut()?.1.push_str(&format!(", {piece}")),
        }
    }
    (!pairs.is_empty()).then_some(pairs)
}

/// The articles in a list of Markdown links.
pub(super) fn links(text: &str) -> Vec<Entry> {
    let mut found: Vec<Map<String, Value>> = Vec::new();
    // Whether lines still belong to the last link's summary.
    let mut open = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some((title, url, rest)) = link(trimmed) {
            if found.iter().any(|f| f["url"] == url.as_str()) {
                open = false;
                continue;
            }
            let rest = plain(rest.trim_start_matches(|c: char| " -–—:|*_".contains(c)));
            let mut fields = Map::new();
            fields.insert("title".into(), plain(&title).into());
            fields.insert("url".into(), url.into());
            fields.insert("summary".into(), rest.into());
            found.push(fields);
            open = true;
        } else if trimmed.starts_with('#') || trimmed == "---" {
            open = false;
        } else if let (true, Some(fields)) = (open, found.last_mut()) {
            if !trimmed.is_empty() && !trimmed.starts_with("![") {
                append(fields, "summary", &plain(trimmed));
            }
        }
    }
    if found.len() < 2 {
        return Vec::new();
    }
    found
        .into_iter()
        .filter_map(|f| item(&Value::Object(f)))
        .collect()
}

/// The first `[text](https://…)` link in a line that isn't an image: its
/// text, its URL, and what follows it.
fn link(line: &str) -> Option<(String, String, &str)> {
    let mut from = 0;
    while let Some(open) = line[from..].find('[').map(|i| i + from) {
        from = open + 1;
        if line[..open].ends_with('!') {
            continue;
        }
        let Some(close) = line[open..].find("](").map(|i| i + open) else {
            break;
        };
        let Some(end) = line[close + 2..].find(')').map(|i| i + close + 2) else {
            break;
        };
        let url = line[close + 2..end].split_whitespace().next().unwrap_or("");
        if url.starts_with("https://") || url.starts_with("http://") {
            return Some((
                line[open + 1..close].to_owned(),
                url.to_owned(),
                &line[end + 1..],
            ));
        }
    }
    None
}

/// Markdown markers dropped from a line of text: list bullets, emphasis,
/// and links reduced to their text.
fn plain(line: &str) -> String {
    let mut line = line.trim();
    for bullet in ["- ", "* ", "+ "] {
        line = line.strip_prefix(bullet).unwrap_or(line);
    }
    if let Some((number, rest)) = line.split_once(". ") {
        if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
            line = rest;
        }
    }
    let mut text = String::new();
    let mut rest = line;
    while let Some((title, url, after)) = link(rest) {
        let at = rest.find(&format!("[{title}]({url}")).unwrap_or(0);
        text.push_str(&rest[..at]);
        text.push_str(&title);
        rest = after;
    }
    text.push_str(rest);
    text.replace("**", "").replace('`', "").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labelled_records_are_articles() {
        let text =
            "Title: First | Site\nURL: https://a.example/1\nPublished: 2026-10-21T00:00:00.000Z\n\
                    Author: N/A\nHighlights:\nFirst line\n\nNote: a colon\n\n---\n\n\
                    Title: Second\nURL: https://b.example/2\nAuthor: Ann\nText: Body";
        let found = labelled(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "First | Site");
        assert_eq!(found[0].url.as_deref(), Some("https://a.example/1"));
        assert_eq!(found[0].author, None);
        assert_eq!(found[0].published_at, Some(1_792_540_800));
        assert_eq!(
            found[0].summary_html.as_deref(),
            Some("<p>First line</p><p>Note: a colon</p>")
        );
        assert_eq!(found[1].author.as_deref(), Some("Ann"));
        assert_eq!(found[1].summary_html.as_deref(), Some("<p>Body</p>"));
        assert!(labelled("URI: `hf://papers/trending`\nOrder: rank").is_empty());
    }

    #[test]
    fn table_rows_are_articles() {
        let text = "| Type | Details |\n|---|---|\n\
                    | paper | title=A \\| B, summary=One, two, three, upvotes=3, web=https://hf.example/1 |\n\
                    | paper | title=No link |\n\n\
                    | Title | Date |\n| :-- | --- |\n| [C](https://c.example/) | 2026-10-24 |";
        let found = tables(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "A | B");
        assert_eq!(found[0].url.as_deref(), Some("https://hf.example/1"));
        assert_eq!(
            found[0].summary_html.as_deref(),
            Some("<p>One, two, three</p>")
        );
        assert_eq!(found[1].title, "C");
        assert_eq!(found[1].url.as_deref(), Some("https://c.example/"));
        assert_eq!(found[1].published_at, Some(1_792_800_000));
    }

    #[test]
    fn a_list_of_links_is_articles() {
        let text = "# Results\n\n1. **[First](https://a.example/1)** - a **bold** start\n   More about it\n\n\
                    ![logo](https://img.example/x.png)\n\
                    - [Second](https://b.example/2 \"title\")\n## Elsewhere\nNot a summary\n\
                    - [First again](https://a.example/1)";
        let found = links(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "First");
        assert_eq!(
            found[0].summary_html.as_deref(),
            Some("<p>a bold start<br>More about it</p>")
        );
        assert_eq!(found[1].url.as_deref(), Some("https://b.example/2"));
        assert_eq!(found[1].summary_html, None);
        // One link in a paragraph isn't a list.
        assert!(links("See [the docs](https://docs.example/) for more.").is_empty());
    }
}
