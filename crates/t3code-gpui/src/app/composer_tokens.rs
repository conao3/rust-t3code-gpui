use super::*;
use gpui_kit::base::input::{InlineToken, InputContent};
use std::sync::LazyLock;

fn content(text: String, instances: &[models::Instance]) -> InputContent {
    static TOKENS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r#"(?m)(^|\s)(@(?:"(?:\\.|[^"\\])*"|[^\s@"]+)|\p{Sc}[a-zA-Z0-9][a-zA-Z0-9:_-]*)"#,
        )
        .expect("composer tokens")
    });
    static FILE_LINKS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)(^|\s)\[((?:\\.|[^\]\\]){0,512})\]\(([^)\s]+)\)")
            .expect("file links")
    });
    static ESCAPES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\\(.)").expect("label escapes"));
    static SCHEME: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[A-Za-z][A-Za-z0-9+.-]*:").expect("URI scheme"));
    let mut content = InputContent::new(text.clone());
    let mut occupied = vec![];
    for capture in FILE_LINKS.captures_iter(&text) {
        let full = capture.get(0).expect("file link");
        let start = full.start() + capture[1].len();
        let end = full.end();
        if end == text.len() || !text[end..].starts_with(char::is_whitespace) {
            continue;
        }
        let Some(path) = decode_path(&capture[3]) else {
            continue;
        };
        let label = ESCAPES.replace_all(&capture[2], "$1").into_owned();
        let windows = path.as_bytes().get(1) == Some(&b':')
            && path
                .as_bytes()
                .get(2)
                .is_some_and(|byte| matches!(byte, b'/' | b'\\'));
        if label != path.rsplit(['/', '\\']).next().unwrap_or_default()
            || (SCHEME.is_match(&path) && !windows)
        {
            continue;
        }
        let source = text[start..end].to_string();
        content = content
            .with_token(
                start..end,
                InlineToken::new(source.clone(), source).with_label(label),
            )
            .expect("valid file token");
        occupied.push(start..end);
    }
    for capture in TOKENS.captures_iter(&text) {
        let token = capture.get(2).expect("token");
        if occupied
            .iter()
            .any(|range| range.start < token.end() && token.start() < range.end)
        {
            continue;
        }
        if token.end() == text.len() || !text[token.end()..].starts_with(char::is_whitespace) {
            continue;
        }
        let source = token.as_str();
        if !source.starts_with('@') && !source.chars().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let label = if let Some(path) = source.strip_prefix('@') {
            path.trim_matches('"')
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .to_string()
        } else {
            source.to_string()
        };
        content = content
            .with_token(
                token.range(),
                InlineToken::new(source.to_string(), source.to_string()).with_label(label),
            )
            .expect("valid composer token");
    }
    static MODELS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"model:([a-zA-Z][a-zA-Z0-9_-]*)/([^\s,"']+)"#).expect("model mentions")
    });
    for capture in MODELS.captures_iter(&text) {
        let found = capture.get(0).expect("model mention");
        if text[..found.start()]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace() && !",([\"'".contains(c))
        {
            continue;
        }
        let mut slug = &capture[2];
        while let Some(last) = slug.chars().next_back() {
            let opening = match last {
                ')' => Some('('),
                ']' => Some('['),
                _ => None,
            };
            if !".;:!?".contains(last)
                && !opening
                    .is_some_and(|open| slug.matches(open).count() < slug.matches(last).count())
            {
                break;
            }
            slug = &slug[..slug.len() - last.len_utf8()];
        }
        let Some(model) = instances
            .iter()
            .find(|instance| instance.instance_id == capture[1])
            .and_then(|instance| instance.models.iter().find(|model| model.slug == slug))
        else {
            continue;
        };
        let end = found.start() + "model:".len() + capture[1].len() + 1 + slug.len();
        if content
            .tokens()
            .iter()
            .any(|token| token.range().start < end && found.start() < token.range().end)
        {
            continue;
        }
        let source = &text[found.start()..end];
        content = content
            .with_token(
                found.start()..end,
                InlineToken::new(source.to_string(), source.to_string())
                    .with_label(model.full_name.clone()),
            )
            .expect("valid model mention");
    }
    content
}

impl Root {
    pub(super) fn composer_content(&self, text: String) -> InputContent {
        let catalog = self
            .composer_target()
            .and_then(|(environment, ..)| self.model_catalogs.get(&environment));
        content(
            text,
            catalog
                .map(|(_, instances)| instances.as_slice())
                .unwrap_or_default(),
        )
    }

    pub(super) fn accept_token(
        &mut self,
        trigger: &completion::Trigger,
        suggestion: &completion::Suggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if trigger.kind == completion::TriggerKind::SlashCommand {
            return false;
        }
        let source = suggestion.insert.trim_end();
        let is_file =
            trigger.kind == completion::TriggerKind::Path && !source.starts_with("model:");
        let text = if is_file {
            file_link(&source[1..])
        } else {
            source.to_string()
        };
        let label = if is_file {
            suggestion
                .label
                .trim_start_matches('@')
                .rsplit('/')
                .next()
                .unwrap_or(&suggestion.label)
                .to_string()
        } else {
            suggestion.label.clone()
        };
        let token = InlineToken::new(text.clone(), text).with_label(label);
        let result = self.conversation.composer.update(cx, |state, cx| {
            state.replace_range_with_token(trigger.start..trigger.end, token, window, cx)?;
            state.replace_text_in_range(None, " ", window, cx);
            Ok::<_, gpui_kit::base::input::InlineTokenError>(())
        });
        if let Err(error) = result {
            self.conversation.message = Some(format!("Could not insert composer token: {error:?}"));
        }
        true
    }
}

fn file_link(path: &str) -> String {
    let label = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]");
    let mut destination = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*';/:@&=+$,".contains(&byte) {
            destination.push(byte as char);
        } else {
            use std::fmt::Write as _;
            write!(destination, "%{byte:02X}").expect("string write");
        }
    }
    format!("[{label}]({destination})")
}

pub(super) fn file_path(source: &str) -> Option<String> {
    if let Some(path) = source.strip_prefix('@') {
        return Some(path.trim_matches('"').to_string());
    }
    let (_, destination) = source.split_once("](")?;
    decode_path(destination.strip_suffix(')')?)
}

fn decode_path(encoded: &str) -> Option<String> {
    let mut bytes = vec![];
    let mut input = encoded.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = (input.next()? as char).to_digit(16)?;
            let low = (input.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::file_link;
    #[test]
    fn model_mentions_keep_balanced_slugs_and_exclude_prose_punctuation() {
        let instances = crate::models::instances(&serde_json::json!({"providers":[{
            "instanceId":"codex", "driver":"codex", "enabled":true,
            "models":[{"slug":"model[1m]", "name":"Model"}]
        }]}));
        let text = "日本語 (model:codex/model[1m]). model:codex/model[1m]! xmodel:codex/model[1m] model:unknown/model[1m]";
        let result = super::content(text.into(), &instances);
        assert_eq!(result.tokens().len(), 2);
        for token in result.tokens() {
            assert_eq!(&text[token.range()], "model:codex/model[1m]");
            assert_eq!(token.token().label().as_ref(), "Model");
        }
    }

    #[test]
    fn file_links_match_reference_serialization() {
        for (path, expected) in [
            (
                "docs/My File (draft).md",
                "[My File (draft).md](docs/My%20File%20%28draft%29.md)",
            ),
            (
                r"C:\repo\src\index.ts",
                "[index.ts](C:%5Crepo%5Csrc%5Cindex.ts)",
            ),
            ("@scope/package.json", "[package.json](@scope/package.json)"),
            ("日本語.md", "[日本語.md](%E6%97%A5%E6%9C%AC%E8%AA%9E.md)"),
        ] {
            assert_eq!(file_link(path), expected);
        }
    }
}
