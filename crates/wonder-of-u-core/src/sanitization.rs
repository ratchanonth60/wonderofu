//! Minimal HTML sanitization helpers.

/// Escapes HTML-special characters.
pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());

    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }

    escaped
}

/// Strips HTML tags from a string without invoking a full parser.
pub fn sanitize_html(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '<' && looks_like_tag_start(chars.get(index + 1).copied()) {
            index += 1;
            let mut quote = None;
            while index < chars.len() {
                let ch = chars[index];
                if let Some(active_quote) = quote {
                    if ch == active_quote {
                        quote = None;
                    }
                } else if matches!(ch, '"' | '\'') {
                    quote = Some(ch);
                } else if ch == '>' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }

        sanitized.push(chars[index]);
        index += 1;
    }

    sanitized
}

fn looks_like_tag_start(next: Option<char>) -> bool {
    matches!(next, Some('/' | '!' | '?')) || next.is_some_and(|ch| ch.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::{escape_html, sanitize_html};

    #[test]
    fn escapes_html_special_characters() {
        assert_eq!(
            escape_html(r#"<tag attr="x">Tom & 'Jerry'</tag>"#),
            "&lt;tag attr=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/tag&gt;"
        );
    }

    #[test]
    fn strips_html_tags_but_keeps_text() {
        assert_eq!(
            sanitize_html(r#"<p>Hello <strong data-x="1>2">world</strong>!</p>"#),
            "Hello world!"
        );
    }
}
