use super::*;
use gpui_kit::base::input::{InlineToken, InputContent};
use std::sync::LazyLock;

pub(super) fn content(text: String) -> InputContent {
    static TOKENS: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r#"(?m)(^|\s)(@(?:"(?:\\.|[^"\\])*"|[^\s@"]+)|[$¥€£][a-zA-Z0-9][a-zA-Z0-9:_-]*)"#,
        )
        .expect("composer tokens")
    });
    let mut content = InputContent::new(text.clone());
    for capture in TOKENS.captures_iter(&text) {
        let token = capture.get(2).expect("token");
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
    content
}

impl Root {
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
        let text = if trigger.kind == completion::TriggerKind::Path
            && source[1..].contains(char::is_whitespace)
        {
            format!(
                "@{}",
                serde_json::to_string(&source[1..]).expect("path string")
            )
        } else {
            source.to_string()
        };
        let label = if trigger.kind == completion::TriggerKind::Path {
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
        let result = self.composer.update(cx, |state, cx| {
            state.replace_range_with_token(trigger.start..trigger.end, token, window, cx)?;
            state.replace_text_in_range(None, " ", window, cx);
            Ok::<_, gpui_kit::base::input::InlineTokenError>(())
        });
        if let Err(error) = result {
            self.message = Some(format!("Could not insert composer token: {error:?}"));
        }
        true
    }
}
