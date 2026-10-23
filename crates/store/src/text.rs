//! What the search index sees of an article: the summary's words, without
//! its markup.

/// Adds one article to the search index: rowid, title, author, body.
pub const INDEX_ARTICLE: &str =
    "INSERT INTO article_fts (rowid, title, author, body) VALUES (?1, ?2, ?3, ?4)";

/// The text of an HTML fragment: tags dropped (with the contents of
/// `script` and `style`), entities decoded, whitespace runs made one space.
/// Words in attributes, link URLs among them, never come through.
pub fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        out.push(' ');
        let tag = &rest[open + 1..];
        let Some(close) = tag.find('>') else {
            rest = "";
            break;
        };
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        rest = &tag[close + 1..];
        // Skip what scripts and styles contain, up to their end tag.
        if name == "script" || name == "style" {
            let end = format!("</{name}");
            rest = find_ignoring_case(rest, &end).map_or("", |i| &rest[i..]);
        }
    }
    out.push_str(rest);
    let decoded = html_escape::decode_html_entities(&out);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// What a person typed, as an FTS5 query that can't be a syntax error:
/// every word must appear, each as a prefix so results follow the typing
/// ("harris" finds Harrisburg). Punctuation splits words, the way the
/// index's tokenizer does. `None` when nothing searchable was typed.
pub fn match_query(typed: &str) -> Option<String> {
    let words: Vec<String> = typed
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_goes_and_text_stays() {
        assert_eq!(
            plain(
                r#"<p>Mayor of <a href="https://harrisburg.example/city">Harrisburg</a> &amp; aides</p><p>Two</p>"#
            ),
            "Mayor of Harrisburg & aides Two"
        );
        assert_eq!(
            plain("before<SCRIPT>var harrisburg = 1;</script>after<style>p{}</style>end"),
            "before after end"
        );
        assert_eq!(plain("cut off <a href"), "cut off");
        assert_eq!(plain("a<br>b"), "a b");
    }

    #[test]
    fn typing_becomes_a_safe_query() {
        assert_eq!(match_query("Harris").as_deref(), Some("\"Harris\"*"));
        assert_eq!(
            match_query("  o'neal -AND \"x").as_deref(),
            Some("\"o\"* \"neal\"* \"AND\"* \"x\"*")
        );
        assert_eq!(match_query(" - * \" "), None);
        assert_eq!(match_query("Köln"), Some("\"Köln\"*".to_owned()));
    }
}
