use std::collections::HashMap;
use std::ops::Range;

use markdown::mdast::Node;

struct SourceRanges {
    inline: Vec<Range<usize>>,
    protected: Vec<Range<usize>>,
    links: Vec<Range<usize>>,
}

impl SourceRanges {
    fn collect(&mut self, node: &Node) {
        if let Some(position) = node.position() {
            let range = position.start.offset..position.end.offset;
            match node {
                Node::Code(_)
                | Node::InlineCode(_)
                | Node::Html(_)
                | Node::Image(_)
                | Node::ImageReference(_)
                | Node::Definition(_) => {
                    self.protected.push(range);
                    return;
                }
                Node::Link(_) | Node::LinkReference(_) => self.links.push(range),
                Node::Paragraph(_) | Node::Heading(_) | Node::TableCell(_) => {
                    self.inline.push(range)
                }
                _ => {}
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                self.collect(child);
            }
        }
    }
}

fn escaped(source: &str, at: usize) -> bool {
    source.as_bytes()[..at]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn label_end(source: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    for (offset, character) in source[start..].char_indices() {
        let at = start + offset;
        if escaped(source, at) {
            continue;
        }
        if character == '[' {
            depth += 1;
        }
        if character == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(at + 1);
            }
        }
    }
    None
}

fn decode(value: &str) -> String {
    let mut result = String::new();
    let mut at = 0;
    while at < value.len() {
        let tail = &value[at..];
        if tail.starts_with('&')
            && let Some(end) = tail.find(';').filter(|end| *end <= 33)
        {
            let entity = &tail[..=end];
            let body = &tail[1..end];
            let numeric = body.strip_prefix('#').and_then(|digits| {
                let (digits, radix, max) = match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => (hex, 16, 6),
                    None => (digits, 10, 7),
                };
                (!digits.is_empty()
                    && digits.len() <= max
                    && digits.chars().all(|digit| digit.is_digit(radix)))
                .then(|| u32::from_str_radix(digits, radix).ok())
                .flatten()
            });
            if let Some(code) = numeric {
                let invalid = matches!(code, 0..=8 | 11 | 14..=31 | 127..=159 | 0xfdd0..=0xfdef)
                    || code & 0xffff >= 0xfffe;
                result.push(if invalid {
                    '\u{fffd}'
                } else {
                    char::from_u32(code).unwrap_or('\u{fffd}')
                });
                at += entity.len();
                continue;
            }
            if body.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                let decoded = html_escape::decode_html_entities(entity);
                if decoded != entity {
                    result.push_str(&decoded);
                    at += entity.len();
                    continue;
                }
            }
        }
        let character = tail.chars().next().expect("in source");
        result.push(character);
        at += character.len_utf8();
    }
    result
}

fn attributes(source: &str, start: usize) -> Option<(usize, HashMap<String, String>)> {
    let bytes = source.as_bytes();
    if bytes.get(start) != Some(&b'{') {
        return None;
    }
    let mut at = start + 1;
    let mut values = HashMap::new();
    loop {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at) == Some(&b'}') {
            return Some((at + 1, values));
        }
        let shorthand = match bytes.get(at) {
            Some(b'#') => Some("id"),
            Some(b'.') => Some("class"),
            _ => None,
        };
        if shorthand.is_some() {
            at += 1;
        }
        let begin = at;
        while bytes
            .get(at)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !b"={}\"'<>`".contains(byte))
        {
            at += 1;
        }
        if at == begin {
            return None;
        }
        let key = shorthand.unwrap_or(&source[begin..at]).to_string();
        if let Some(shorthand) = shorthand {
            let value = decode(&source[begin..at]);
            if shorthand == "class" {
                values
                    .entry(key)
                    .and_modify(|old: &mut String| {
                        old.push(' ');
                        old.push_str(&value);
                    })
                    .or_insert(value);
            } else {
                values.insert(key, value);
            }
            continue;
        }
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        let value = if bytes.get(at) == Some(&b'=') {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            match bytes.get(at) {
                Some(b'\'' | b'"') => {
                    let quote = bytes[at];
                    at += 1;
                    let begin = at;
                    while bytes.get(at).is_some_and(|byte| *byte != quote) {
                        at += 1;
                    }
                    if bytes.get(at) != Some(&quote) {
                        return None;
                    }
                    let result = decode(&source[begin..at]);
                    at += 1;
                    if bytes
                        .get(at)
                        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'}')
                    {
                        return None;
                    }
                    result
                }
                Some(_) => {
                    let begin = at;
                    while bytes
                        .get(at)
                        .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'}')
                    {
                        if b"\"'<=`".contains(&bytes[at]) {
                            return None;
                        }
                        at += 1;
                    }
                    if begin == at {
                        return None;
                    }
                    decode(&source[begin..at])
                }
                None => return None,
            }
        } else {
            String::new()
        };
        values.insert(key, value);
    }
}

fn positive_integer(value: &str) -> Option<u64> {
    let value = value.trim();
    let number = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()? as f64
    } else if let Some(binary) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        u64::from_str_radix(binary, 2).ok()? as f64
    } else if let Some(octal) = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
    {
        u64::from_str_radix(octal, 8).ok()? as f64
    } else {
        value.parse::<f64>().ok()?
    };
    (number.is_finite() && number > 0. && number <= 9_007_199_254_740_991. && number.fract() == 0.)
        .then_some(number as u64)
}

fn file_citation(attributes: &HashMap<String, String>) -> Option<String> {
    let path = attributes.get("path")?.trim();
    if path.is_empty() {
        return None;
    }
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    let label = normalized
        .rsplit('/')
        .next()
        .filter(|label| !label.is_empty())
        .unwrap_or("File");
    let label = label.chars().fold(String::new(), |mut result, character| {
        if "\\[]*_`<&".contains(character) {
            result.push('\\');
        }
        result.push(character);
        result
    });
    let mut destination = path
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('?', "%3F");
    if let Some(line) = attributes
        .get("line_range_start")
        .and_then(|line| positive_integer(line))
    {
        destination.push_str(&format!("#L{line}"));
    }
    let destination = destination
        .replace('\\', "%5C")
        .replace('<', "%3C")
        .replace('>', "%3E")
        .replace('\r', "%0D")
        .replace('\n', "%0A");
    Some(format!("[{label}](<{destination}>)"))
}

fn template(attributes: &HashMap<String, String>) -> Option<String> {
    let kind = match attributes.get("artifact_kind")?.as_str() {
        "document" => "Document",
        "presentation" => "Presentation",
        "spreadsheet" => "Spreadsheet",
        "site" => "Site",
        "google-docs" => "Google Doc",
        "google-slides" => "Google Slides",
        "google-sheets" => "Google Sheet",
        "image" => "Image",
        "email" => "Email",
        "slack" => "Slack",
        _ => return None,
    };
    let display_name = attributes.get("display_name")?.trim();
    let skill_directory = attributes.get("skill_directory")?;
    let bytes = skill_directory.as_bytes();
    let posix = skill_directory.starts_with('/') && !skill_directory.starts_with("//");
    let drive = bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(1) == Some(&b':')
        && bytes.get(2).is_some_and(|byte| b"/\\".contains(byte));
    let unc = skill_directory
        .strip_prefix("\\\\")
        .map(|tail| (tail, '\\'))
        .or_else(|| skill_directory.strip_prefix("//").map(|tail| (tail, '/')))
        .is_some_and(|(tail, separator)| {
            let mut parts = tail.split(separator);
            parts.next().is_some_and(|part| !part.is_empty())
                && parts.next().is_some_and(|part| !part.is_empty())
        });
    if display_name.is_empty()
        || !(posix || drive || unc)
        || !attributes
            .get("skill_name")?
            .starts_with("artifact-template-")
        || attributes
            .get("gallery_kind")
            .is_some_and(|kind| !matches!(kind.as_str(), "imagegen" | "product-design"))
    {
        return None;
    }
    Some(format!("{display_name} ({kind} template)"))
}

pub fn for_copy(source: &str) -> anyhow::Result<String> {
    if ![
        ":codex-file-citation",
        ":codex-followup",
        "::artifact-template",
    ]
    .iter()
    .any(|name| source.contains(name))
    {
        return Ok(source.to_string());
    }
    let tree = markdown::to_mdast(source, &markdown::ParseOptions::default())
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut ranges = SourceRanges {
        inline: Vec::new(),
        protected: Vec::new(),
        links: Vec::new(),
    };
    ranges.collect(&tree);
    let mut replacements = Vec::new();
    for range in &ranges.inline {
        let mut at = range.start;
        while at < range.end {
            if source.as_bytes()[at] != b':'
                || escaped(source, at)
                || at > 0 && source.as_bytes()[at - 1] == b':'
                || ranges.protected.iter().any(|range| range.contains(&at))
            {
                at += source[at..].chars().next().expect("in source").len_utf8();
                continue;
            }
            let Some((name, block)) = [
                (":codex-file-citation", false),
                (":codex-followup", false),
                ("::artifact-template", true),
            ]
            .into_iter()
            .find(|(name, _)| source[at..].starts_with(name)) else {
                at += 1;
                continue;
            };
            let mut end = at + name.len();
            if source[end..].chars().next().is_some_and(|character| {
                character.is_alphanumeric() || character == '-' || character == '_'
            }) {
                at += 1;
                continue;
            }
            let label = if source.as_bytes().get(end) == Some(&b'[') {
                let Some(label_end) = label_end(source, end) else {
                    at += 1;
                    continue;
                };
                let label = &source[end + 1..label_end - 1];
                end = label_end;
                Some(label)
            } else {
                None
            };
            let Some((end, attributes)) = attributes(source, end) else {
                at = end;
                continue;
            };
            if end > range.end {
                at += 1;
                continue;
            }
            let replacement = match name {
                ":codex-file-citation" if !ranges.links.iter().any(|range| range.contains(&at)) => {
                    file_citation(&attributes)
                }
                ":codex-followup" => label
                    .filter(|label| {
                        attributes
                            .get("prompt")
                            .is_some_and(|prompt| !prompt.trim().is_empty())
                            && markdown::to_mdast(label, &markdown::ParseOptions::default())
                                .is_ok_and(|node| !node.to_string().trim().is_empty())
                    })
                    .map(str::to_string),
                "::artifact-template"
                    if block
                        && source[range.start..at]
                            .rsplit('\n')
                            .next()
                            .is_some_and(|line| line.trim().is_empty())
                        && source[end..range.end]
                            .lines()
                            .next()
                            .is_none_or(|line| line.trim().is_empty()) =>
                {
                    template(&attributes)
                }
                _ => None,
            };
            if let Some(replacement) = replacement {
                replacements.push((at..end, replacement));
            }
            at = end;
        }
    }
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = source.to_string();
    for (range, replacement) in replacements {
        result.replace_range(range, &replacement);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_copy_oracle() {
        let oracle: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/directive-copy-oracle.json"))
                .unwrap();
        let failures: Vec<_> = oracle["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|case| {
                let source = case["source"].as_str().unwrap();
                let expected = case["expected"].as_str().unwrap();
                let actual = for_copy(source).unwrap();
                (actual != expected)
                    .then(|| format!("{source:?}\nexpected={expected:?}\nactual={actual:?}"))
            })
            .collect();
        assert!(
            failures.is_empty(),
            "{} mismatches:\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
    const FILE: &str = ":codex-file-citation{path=\"outputs/report.xlsx\" purpose=\"output\"}";
    const FOLLOW: &str =
        ":codex-followup[Prepare print version]{prompt=\"Prepare the document for printing.\"}";
    const TEMPLATE: &str = "::artifact-template{skill_name=\"artifact-template-hello-world\" skill_directory=\"/Users/test/.codex/skills/artifact-template-hello-world\" display_name=\"Hello World\" artifact_kind=\"document\"}";
    #[test]
    fn reference_directive_copy_fixture() {
        assert_eq!(
            for_copy(&format!("Created {FILE}.\n\n{TEMPLATE}\n\n- {FOLLOW}")).unwrap(),
            "Created [report.xlsx](<outputs/report.xlsx>).\n\nHello World (Document template)\n\n- Prepare print version"
        );
    }
    #[test]
    fn literal_code_escaped_unknown_and_malformed_examples_stay_literal() {
        for source in [
            format!("`{FILE}`"),
            format!("```\n{FILE}\n```"),
            format!("\\{FILE}"),
            ":codex-file-citation{purpose=\"output\"}".into(),
            ":codex-file-citation-extra{path=a}".into(),
            ":codex-followup[ ]{prompt=p}".into(),
            "::artifact-template{display_name=\"Hello World\"}".into(),
        ] {
            assert_eq!(for_copy(&source).unwrap(), source);
        }
    }
    #[test]
    fn citations_inside_links_stay_literal_and_copy_labels_keep_markdown() {
        let source = format!(
            "[{FILE}](https://example.com)\n\n:codex-followup[**Prepare** `print`]{{prompt=\"print\"}}"
        );
        assert_eq!(
            for_copy(&source).unwrap(),
            format!("[{FILE}](https://example.com)\n\n**Prepare** `print`")
        );
    }
    #[test]
    fn unicode_before_directives_keeps_source_byte_offsets() {
        assert_eq!(
            for_copy(&format!("資料🙂 {FILE}")).unwrap(),
            "資料🙂 [report.xlsx](<outputs/report.xlsx>)"
        );
    }

    #[test]
    fn unicode_paths_entities_and_line_numbers_match_citation_links() {
        assert_eq!(
            for_copy(":codex-file-citation{path=\"資料/a&#38;b#?.md\" line_range_start=\"1e2\"}")
                .unwrap(),
            "[a\\&b#?.md](<資料/a&b%23%3F.md#L100>)"
        );
    }
}
