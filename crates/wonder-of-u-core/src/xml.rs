//! Small XML string helpers.

/// Returns the contents of the first matching XML tag.
pub fn parse_tag_contents<'a>(tag: &str, input: &'a str) -> Option<&'a str> {
    let (content_start, _) = find_open_tag(tag, input)?;
    let closing_tag = format!("</{tag}>");
    let close_start = input[content_start..].find(&closing_tag)? + content_start;
    Some(&input[content_start..close_start])
}

/// Returns the first complete matching XML block, including the wrapper tags.
pub fn extract_xml_block<'a>(tag: &str, input: &'a str) -> Option<&'a str> {
    let (content_start, start) = find_open_tag(tag, input)?;
    let closing_tag = format!("</{tag}>");
    let close_start = input[content_start..].find(&closing_tag)? + content_start;
    let close_end = close_start + closing_tag.len();
    Some(&input[start..close_end])
}

fn find_open_tag(tag: &str, input: &str) -> Option<(usize, usize)> {
    let needle = format!("<{tag}");
    let mut search_from = 0;

    while let Some(relative_start) = input[search_from..].find(&needle) {
        let start = search_from + relative_start;
        let after_name = start + needle.len();
        let boundary = input[after_name..].chars().next()?;
        if boundary != '>' && !boundary.is_ascii_whitespace() {
            search_from = after_name;
            continue;
        }

        let open_end = find_tag_end(input, after_name)?;
        return Some((open_end, start));
    }

    None
}

fn find_tag_end(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, ch) in input[start..].char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '>' => return Some(start + offset + ch.len_utf8()),
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{extract_xml_block, parse_tag_contents};

    #[test]
    fn parses_tag_contents_with_attributes() {
        let input = r#"<root><item id="1">value</item></root>"#;
        assert_eq!(parse_tag_contents("item", input), Some("value"));
    }

    #[test]
    fn extracts_full_xml_blocks() {
        let input = "<item>value</item><item>other</item>";
        assert_eq!(extract_xml_block("item", input), Some("<item>value</item>"));
    }
}
