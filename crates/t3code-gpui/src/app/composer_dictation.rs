use super::*;
use crate::dictation::Recording;

#[derive(Default)]
pub(super) struct Dictation {
    pub(super) recording: Option<Recording>,
    pub(super) busy: bool,
    target: Option<String>,
}

impl Root {
    pub(super) fn toggle_dictation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation.dictation.busy || self.composer_composing(window, cx) {
            return;
        }
        let Some((environment, ..)) = self.composer_target() else {
            return;
        };
        let Some(connection) = self.connection(&environment) else {
            return;
        };
        let Some(config) = self
            .config
            .environments
            .iter()
            .find(|env| env.label == environment)
        else {
            return;
        };
        let api = Api::new(&config.http_base_url, &config.token);
        let runtime = self.runtime.clone();
        let target = self.composer_key();
        if let Some(recording) = self.conversation.dictation.recording.take() {
            if self.conversation.dictation.target != target {
                drop(recording);
                self.conversation.message =
                    Some("Dictation cancelled because the composer changed.".into());
                cx.notify();
                return;
            }
            let settings = match std::fs::read(&self.settings_path) {
                Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(anyhow::Error::from),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
                Err(error) => Err(error.into()),
            };
            let settings = match settings {
                Ok(settings) => settings,
                Err(error) => {
                    self.conversation.message =
                        Some(format!("Could not read speech preferences: {error:#}"));
                    cx.notify();
                    return;
                }
            };
            self.conversation.dictation.busy = true;
            let mut options = json!({
                "speechLanguage": settings["speechLanguage"].as_str().unwrap_or("auto"),
                "speechTranslateToEnglish": settings["speechTranslateToEnglish"].as_bool().unwrap_or(false),
                "speechCustomWords": settings["speechCustomWords"].as_array().cloned().unwrap_or_default(),
                "speechRemoveFillerWords": settings["speechRemoveFillerWords"].as_bool().unwrap_or(true),
                "speechSilenceFiltering": settings["speechSilenceFiltering"].as_bool().unwrap_or(true)
            });
            if let Some(cue) = settings
                .get("speechCorrectionWord")
                .filter(|value| value.is_string())
            {
                options["speechCorrectionWord"] = cue.clone();
            }
            self.spawn_pane_in(window, cx, async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    let pcm = tokio::task::spawn_blocking(move || recording.finish()).await??;
                    let body = crate::dictation::encode(&pcm, &options)?;
                    api.post_binary_json("/api/voice/transcribe", body).await
                })
                .await;
                let _ = this.update_in(cx, |root, window, cx| {
                    root.conversation.dictation.busy = false;
                    match result {
                        Ok(result) => {
                            if let Some(text) = result["text"].as_str() {
                                if root.composer_key() == target {
                                    root.conversation.composer.update(cx, |state, cx| {
                                        state.replace_text_in_range(None, text, window, cx)
                                    });
                                    root.stash_composer(cx);
                                } else {
                                    root.conversation.message = Some(format!(
                                        "Dictation completed for another composer: {text}"
                                    ));
                                }
                            } else {
                                root.conversation.message =
                                    Some("Speech server returned no transcript.".into());
                            }
                        }
                        Err(error) => {
                            root.conversation.message = Some(format!("Dictation failed: {error:#}"))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        } else {
            self.conversation.dictation.busy = true;
            self.spawn_pane(cx, async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    let status = api.get("/api/voice/status").await?;
                    anyhow::ensure!(
                        status["state"] == "ready",
                        "Speech model is not ready on {}. Configure voice on this host.",
                        connection.label()
                    );
                    Recording::start()
                })
                .await;
                let _ = this.update(cx, |root, cx| {
                    root.conversation.dictation.busy = false;
                    match result {
                        Ok(recording) if root.composer_key() != target => drop(recording),
                        Ok(recording) => {
                            root.conversation.dictation.recording = Some(recording);
                            root.conversation.dictation.target = target;
                        }
                        Err(error) => {
                            root.conversation.message = Some(format!("Dictation failed: {error:#}"))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }
}
