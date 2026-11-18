use super::*;
use crate::dictation::Recording;

#[derive(Default)]
pub(super) struct Dictation {
    pub(super) recording: Option<Recording>,
    pub(super) busy: bool,
    stream: Option<t3_client::speech::Stream>,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    request: u64,
    status: String,
    preview: String,
    target: Option<String>,
}

impl Root {
    pub(super) fn cancel_dictation(&mut self, cx: &mut Context<Self>) {
        let request = self.conversation.dictation.request + 1;
        if let Some(cancel) = self.conversation.dictation.cancel.take() {
            let _ = cancel.send(());
        }
        self.conversation.dictation = Dictation {
            request,
            ..Default::default()
        };
        cx.notify();
    }

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
        let request = self.conversation.dictation.request + 1;
        self.conversation.dictation.request = request;
        if let Some(recording) = self.conversation.dictation.recording.take() {
            if self.conversation.dictation.target != target {
                drop(recording);
                self.cancel_dictation(cx);
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
                    self.cancel_dictation(cx);
                    self.conversation.message =
                        Some(format!("Could not read speech preferences: {error:#}"));
                    cx.notify();
                    return;
                }
            };
            self.conversation.dictation.busy = true;
            self.conversation.dictation.status = "Transcribing".into();
            let stream = self.conversation.dictation.stream.take();
            let (cancel, cancelled) = tokio::sync::oneshot::channel();
            self.conversation.dictation.cancel = Some(cancel);
            let options = json!({
                "speechLanguage": settings["speechLanguage"].as_str().unwrap_or("auto"),
                "speechTranslateToEnglish": settings["speechTranslateToEnglish"].as_bool().unwrap_or(false),
                "speechCustomWords": settings["speechCustomWords"].as_array().cloned().unwrap_or_default(),
                "speechRemoveFillerWords": settings["speechRemoveFillerWords"].as_bool().unwrap_or(true),
                "speechSilenceFiltering": settings["speechSilenceFiltering"].as_bool().unwrap_or(true)
            });
            self.spawn_pane_in(window, cx, async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    tokio::select! {
                        _ = cancelled => anyhow::bail!("Voice transcription was cancelled."),
                        result = async move {
                            let pcm = tokio::task::spawn_blocking(move || recording.finish()).await??;
                            if let Some(stream) = stream { return Ok(json!({"text":stream.finish().await?})); }
                            let body = crate::dictation::encode(&pcm, &options)?;
                            api.post_binary_json("/api/voice/transcribe", body).await
                        } => result
                    }
                })
                .await;
                let _ = this.update_in(cx, |root, window, cx| {
                    if root.conversation.dictation.request != request { return; }
                    root.conversation.dictation.busy = false;
                    root.conversation.dictation.cancel = None;
                    root.conversation.dictation.status.clear();
                    root.conversation.dictation.preview.clear();
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
            self.conversation.dictation.status = "Preparing".into();
            let (cancel, cancelled) = tokio::sync::oneshot::channel();
            self.conversation.dictation.cancel = Some(cancel);
            let settings = self.settings.values.clone();
            let options = json!({
                "speechLanguage":settings["speechLanguage"].as_str().unwrap_or("auto"),
                "speechTranslateToEnglish":settings["speechTranslateToEnglish"].as_bool().unwrap_or(false),
                "speechCustomWords":settings["speechCustomWords"].as_array().cloned().unwrap_or_default(),
                "speechRemoveFillerWords":settings["speechRemoveFillerWords"].as_bool().unwrap_or(true),
                "speechSilenceFiltering":settings["speechSilenceFiltering"].as_bool().unwrap_or(true)
            });
            self.spawn_pane(cx, async move |this, cx| {
                let result = on_runtime(&runtime, async move {
                    tokio::select! {
                        _ = cancelled => anyhow::bail!("Voice transcription was cancelled."),
                        result = async move {
                    let status = api.get("/api/voice/status").await?;
                    anyhow::ensure!(
                        status["state"] == "ready",
                        "Speech model is not ready on {}. Configure voice on this host.",
                        connection.label()
                    );
                    let (updates, receiver) = tokio::sync::mpsc::unbounded_channel();
                    let stream = if status["supportsStreaming"] == true
                        || status["supportsBufferedStreaming"] == true
                    {
                        Some(t3_client::speech::Stream::start(&api, options, updates).await?)
                    } else {
                        None
                    };
                    let recording =
                        Recording::start(stream.as_ref().map(t3_client::speech::Stream::sender))?;
                    Ok((recording, stream, receiver))
                        } => result
                    }
                })
                .await;
                let _ = this.update(cx, |root, cx| {
                    if root.conversation.dictation.request != request {
                        return;
                    }
                    root.conversation.dictation.busy = false;
                    root.conversation.dictation.cancel = None;
                    match result {
                        Ok(result) if root.composer_key() != target => {
                            drop(result);
                            root.cancel_dictation(cx);
                        }
                        Ok((recording, stream, mut receiver)) => {
                            root.conversation.dictation.recording = Some(recording);
                            root.conversation.dictation.stream = stream;
                            root.conversation.dictation.target = target;
                            root.conversation.dictation.status = "Recording".into();
                            root.spawn_pane(cx, async move |this, cx| {
                                while let Some(text) = receiver.recv().await {
                                    let alive = this
                                        .update(cx, |root, cx| {
                                            if root.conversation.dictation.request != request {
                                                return false;
                                            }
                                            if let Some(error) = text["error"].as_str() {
                                                root.cancel_dictation(cx);
                                                root.conversation.message = Some(error.to_string());
                                                return false;
                                            }
                                            root.conversation.dictation.preview = format!(
                                                "{}{}",
                                                text["committed"].as_str().unwrap_or_default(),
                                                text["tentative"].as_str().unwrap_or_default()
                                            );
                                            cx.notify();
                                            true
                                        })
                                        .unwrap_or(false);
                                    if !alive {
                                        break;
                                    }
                                }
                            })
                            .detach();
                        }
                        Err(error) => {
                            root.conversation.dictation.status.clear();
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

    pub(super) fn render_dictation(&self, cx: &Context<Self>) -> AnyElement {
        let dictation = &self.conversation.dictation;
        if dictation.status.is_empty() {
            return Button::new("dictation")
                .icon(IconName::Mic)
                .ghost()
                .size(px(28.))
                .accessibility_label("Start voice input")
                .tooltip("Start voice input")
                .on_click(
                    self.pane_listener(cx, |root, _, window, cx| root.toggle_dictation(window, cx)),
                )
                .into_any_element();
        }
        h_flex()
            .gap_1()
            .rounded_full()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("cancel-voice-input")
                    .icon(IconName::Close)
                    .ghost()
                    .size(px(28.))
                    .accessibility_label("Cancel voice input")
                    .on_click(self.pane_listener(cx, |root, _, _, cx| root.cancel_dictation(cx))),
            )
            .child(
                div()
                    .id("voice-input-status")
                    .role(accesskit::Role::Status)
                    .aria_label(dictation.status.clone())
                    .text_sm()
                    .child(text!(dictation.status.clone())),
            )
            .when(!dictation.preview.is_empty(), |row| {
                row.child(
                    div()
                        .max_w(px(200.))
                        .truncate()
                        .text_sm()
                        .child(text!(dictation.preview.clone())),
                )
            })
            .child(
                Button::new("finish-voice-input")
                    .icon(IconName::Check)
                    .ghost()
                    .size(px(28.))
                    .accessibility_label(if dictation.recording.is_some() {
                        "Finish voice input".into()
                    } else {
                        dictation.status.clone()
                    })
                    .disabled(dictation.busy)
                    .on_click(self.pane_listener(cx, |root, _, window, cx| {
                        root.toggle_dictation(window, cx)
                    })),
            )
            .into_any_element()
    }
}

impl Dictation {
    pub(super) fn stamp(&self) -> (String, String) {
        (self.status.clone(), self.preview.clone())
    }
}
