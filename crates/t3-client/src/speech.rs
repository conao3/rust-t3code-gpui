use anyhow::{Context as _, Result, bail, ensure};
use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use std::collections::VecDeque;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub enum Input {
    Audio(Vec<u8>),
    Finish,
}

pub struct Stream {
    sender: mpsc::UnboundedSender<Input>,
    task: Option<tokio::task::JoinHandle<Result<String>>>,
}

impl Stream {
    pub async fn start(
        api: &crate::http::Api,
        options: Value,
        updates: mpsc::UnboundedSender<Value>,
    ) -> Result<Self> {
        let ticket = api.post("/api/auth/websocket-ticket/media", None).await?;
        let ticket = ticket["ticket"]
            .as_str()
            .context("Speech WebSocket ticket missing")?;
        let mut url = reqwest::Url::parse(&api.origin)?;
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme)
            .map_err(|()| anyhow::anyhow!("Invalid speech URL"))?;
        url.set_path("/ws/voice");
        url.query_pairs_mut().append_pair("wsTicket", ticket);
        Self::connect(url.as_str(), options, updates).await
    }

    async fn connect(
        url: &str,
        options: Value,
        updates: mpsc::UnboundedSender<Value>,
    ) -> Result<Self> {
        let (mut socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .context("Could not connect to live transcription.")?;
        socket
            .send(Message::Text(
                json!({"type":"start","options":options}).to_string().into(),
            ))
            .await?;
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let errors = updates.clone();
        let task = tokio::spawn(async move {
            let result: Result<String> = async move {
            let mut queued = VecDeque::<Vec<u8>>::new();
            let mut in_flight = VecDeque::<usize>::new();
            let mut bytes = 0usize;
            let mut ready = false;
            let mut finishing = false;
            let mut finish_sent = false;
            let mut revision = -1;
            let mut deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
            loop {
                if ready {
                    while in_flight.len() < 2
                        && let Some(chunk) = queued.pop_front()
                    {
                        in_flight.push_back(chunk.len());
                        socket.send(Message::Binary(chunk.into())).await?;
                        deadline =
                            tokio::time::Instant::now() + std::time::Duration::from_secs(120);
                    }
                    if finishing && queued.is_empty() && in_flight.is_empty() && !finish_sent {
                        socket
                            .send(Message::Text(json!({"type":"finish"}).to_string().into()))
                            .await?;
                        finish_sent = true;
                        deadline =
                            tokio::time::Instant::now() + std::time::Duration::from_secs(120);
                    }
                }
                tokio::select! {
                    input = receiver.recv(), if !finishing => match input {
                        Some(Input::Audio(audio)) => {
                            ensure!(audio.len().is_multiple_of(4), "Microphone PCM is incomplete");
                            ensure!(bytes + audio.len() <= 16_000 * 4 * 300, "Voice recordings are limited to five minutes.");
                            bytes += audio.len();
                            queued.extend(audio.chunks(64_000).map(<[u8]>::to_vec));
                        }
                        Some(Input::Finish) => finishing = true,
                        None => bail!("Voice transcription was cancelled."),
                    },
                    message = socket.next() => {
                        let message = message.context("The voice connection closed. Please record again.")??;
                        let Message::Text(text) = message else {
                            if matches!(message, Message::Close(_)) { bail!("The voice connection closed. Please record again."); }
                            continue;
                        };
                        let message: Value = serde_json::from_str(&text)?;
                        match message["type"].as_str() {
                            Some("ready") if !ready => ready = true,
                            Some("update") => {
                                let next_revision = message["revision"].as_i64().context("Unexpected speech stream response.")?;
                                ensure!(next_revision >= revision, "Unexpected speech stream response.");
                                bytes -= in_flight.pop_front().context("Unexpected speech stream response.")?;
                                revision = next_revision;
                                if !message["text"].is_null() { let _ = updates.send(message["text"].clone()); }
                            }
                            Some("finished") if finish_sent => {
                                let text = message["text"].as_str().context("Unexpected speech stream response.")?.to_string();
                                socket.close(None).await?;
                                return Ok(text);
                            }
                            Some("error") => bail!("{}", message["message"].as_str().unwrap_or("Voice input failed")),
                            _ => bail!("Unexpected speech stream response."),
                        }
                        deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
                    }
                    _ = tokio::time::sleep_until(deadline), if !ready || !in_flight.is_empty() || finish_sent => {
                        bail!("Live transcription stopped responding.");
                    }
                }
            }
            }.await;
            if let Err(error) = &result {
                let _ = errors.send(json!({"error":error.to_string()}));
            }
            result
        });
        Ok(Self {
            sender,
            task: Some(task),
        })
    }

    pub fn sender(&self) -> mpsc::UnboundedSender<Input> {
        self.sender.clone()
    }

    pub async fn finish(mut self) -> Result<String> {
        self.sender
            .send(Input::Finish)
            .context("Voice connection closed")?;
        let result = self
            .task
            .as_mut()
            .context("Speech stream already finished")?
            .await?;
        self.task.take();
        result
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_waits_for_acknowledgements_and_finishes_after_capture() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (connection, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(connection).await.unwrap();
            let start = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&start).unwrap(),
                json!({"type":"start","options":{"speechLanguage":"ja"}})
            );
            socket
                .send(Message::Text(json!({"type":"ready"}).to_string().into()))
                .await
                .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    socket.next().await.unwrap().unwrap().into_data().len(),
                    64_000
                );
            }
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(30), socket.next())
                    .await
                    .is_err()
            );
            for revision in 0..3 {
                socket.send(Message::Text(json!({"type":"update","revision":revision,"text":{"committed":"日本語","tentative":""}}).to_string().into())).await.unwrap();
                if revision == 0 {
                    assert_eq!(
                        socket.next().await.unwrap().unwrap().into_data().len(),
                        64_000
                    );
                }
            }
            let finish = socket.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&finish).unwrap(),
                json!({"type":"finish"})
            );
            socket
                .send(Message::Text(
                    json!({"type":"finished","text":"日本語の入力"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let _ = socket.next().await;
        });
        let (updates, mut receiver) = mpsc::unbounded_channel();
        let stream = Stream::connect(
            &format!("ws://{address}"),
            json!({"speechLanguage":"ja"}),
            updates,
        )
        .await
        .unwrap();
        stream
            .sender()
            .send(Input::Audio(vec![0; 192_000]))
            .unwrap();
        assert_eq!(stream.finish().await.unwrap(), "日本語の入力");
        assert_eq!(receiver.recv().await.unwrap()["committed"], "日本語");
        server.await.unwrap();
    }
}
