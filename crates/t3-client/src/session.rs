use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;

use crate::config::Environment;
use crate::http::Api;
use crate::projection::{Applied, ShellState, ThreadState, apply_config};
use crate::rpc::{RpcClient, RpcError};

const DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_LIMIT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Status {
        environment: String,
    },
    Shell {
        environment: String,
    },
    Config {
        environment: String,
    },
    Thread {
        environment: String,
        thread_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Start,
    Queue,
    Steer,
}

pub struct Launch<'a> {
    pub thread_id: &'a str,
    pub project_id: &'a str,
    pub title: &'a str,
    pub model: Value,
    pub runtime_mode: &'a str,
    pub interaction_mode: &'a str,
    pub attachments: Vec<Value>,
    pub text: &'a str,
}

pub struct Connection {
    label: String,
    http_base_url: String,
    api: Api,
    rpc: AsyncMutex<Option<RpcClient>>,
    status: Mutex<ConnectionStatus>,
    environment_id: Mutex<Option<String>>,
    shell: Arc<Mutex<ShellState>>,
    config: Arc<Mutex<Option<Value>>>,
    changes: async_channel::Sender<Change>,
}

impl Connection {
    pub fn new(environment: &Environment, changes: async_channel::Sender<Change>) -> Arc<Self> {
        Arc::new(Self {
            label: environment.label.clone(),
            http_base_url: environment.http_base_url.trim_end_matches('/').to_string(),
            api: Api::new(&environment.http_base_url, &environment.token),
            rpc: AsyncMutex::new(None),
            status: Mutex::new(ConnectionStatus::Connecting),
            environment_id: Mutex::new(None),
            shell: Arc::new(Mutex::new(ShellState::default())),
            config: Arc::new(Mutex::new(None)),
            changes,
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn http_base_url(&self) -> &str {
        &self.http_base_url
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status.lock().expect("status lock").clone()
    }

    pub fn environment_id(&self) -> Option<String> {
        self.environment_id
            .lock()
            .expect("environment id lock")
            .clone()
    }

    pub fn shell(&self) -> Arc<Mutex<ShellState>> {
        self.shell.clone()
    }

    pub fn config(&self) -> Arc<Mutex<Option<Value>>> {
        self.config.clone()
    }

    pub fn thread_url(&self, thread_id: &str) -> Option<String> {
        let environment_id = self.environment_id()?;
        Some(format!(
            "{}/{environment_id}/{thread_id}",
            self.http_base_url
        ))
    }

    fn notify(&self, change: Change) {
        let _ = self.changes.try_send(change);
    }

    fn set_status(&self, status: ConnectionStatus) {
        let mut held = self.status.lock().expect("status lock");
        if *held != status {
            *held = status;
            drop(held);
            self.notify(Change::Status {
                environment: self.label.clone(),
            });
        }
    }

    pub async fn describe(&self) -> Result<Value> {
        let descriptor = self.api.get("/.well-known/t3/environment").await?;
        if let Some(id) = descriptor.get("environmentId").and_then(Value::as_str) {
            *self.environment_id.lock().expect("environment id lock") = Some(id.to_string());
        }
        Ok(descriptor)
    }

    pub async fn auth_session(&self) -> Result<Value> {
        self.api.get("/api/auth/session").await
    }

    async fn rpc(&self) -> Result<RpcClient> {
        let mut held = self.rpc.lock().await;
        if let Some(client) = held.as_ref().filter(|c| !c.is_closed()) {
            return Ok(client.clone());
        }
        let client = RpcClient::connect(&self.api).await?;
        *held = Some(client.clone());
        Ok(client)
    }

    pub async fn dispatch(&self, mut command: Value) -> Result<u64> {
        if command.get("commandId").is_none() {
            command["commandId"] = json!(uuid::Uuid::new_v4().to_string());
        }
        let command_type = command["type"].as_str().unwrap_or("command").to_string();
        let result = self
            .rpc()
            .await?
            .call("orchestration.dispatchCommand", command, DISPATCH_TIMEOUT)
            .await
            .map_err(|e| match e {
                RpcError::Failed { message, .. } => {
                    anyhow!("T3 rejected {command_type}: {message}")
                }
                other => other.into(),
            })?;
        result
            .get("sequence")
            .and_then(Value::as_u64)
            .with_context(|| format!("T3 returned no sequence for {command_type}"))
    }

    pub async fn launch_thread(&self, launch: Launch<'_>) -> Result<String> {
        let payload = json!({
            "commandId": uuid::Uuid::new_v4().to_string(),
            "creationSource": "web",
            "threadId": launch.thread_id,
            "projectId": launch.project_id,
            "title": launch.title,
            "generateTitle": true,
            "modelSelection": launch.model,
            "runtimeMode": launch.runtime_mode,
            "interactionMode": launch.interaction_mode,
            "workspaceStrategy": {"type": "root"},
            "initialMessage": {
                "messageId": uuid::Uuid::new_v4().to_string(),
                "text": launch.text,
                "attachments": launch.attachments,
            },
        });
        let result = self
            .rpc()
            .await?
            .call("orchestration.launchThread", payload, DISPATCH_TIMEOUT)
            .await
            .map_err(|e| match e {
                RpcError::Failed { message, .. } => {
                    anyhow!("T3 rejected the new thread: {message}")
                }
                other => other.into(),
            })?;
        result
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("T3 returned no thread id for the new thread")
    }

    pub async fn send_message(
        &self,
        thread_id: &str,
        text: &str,
        attachments: Vec<Value>,
        delivery: Delivery,
    ) -> Result<u64> {
        let mut command = json!({
            "type": "message.dispatch",
            "threadId": thread_id,
            "messageId": uuid::Uuid::new_v4().to_string(),
            "text": text,
            "attachments": attachments,
            "createdBy": "user",
            "creationSource": "web",
        });
        match delivery {
            Delivery::Start => {
                command["deliveryIntent"] = json!("auto");
                command["dispatchMode"] = json!({"type": "start_immediately"});
            }
            Delivery::Steer => {
                command["deliveryIntent"] = json!("steer");
                command["dispatchMode"] = json!({"type": "start_immediately"});
            }
            Delivery::Queue => command["dispatchMode"] = json!({"type": "queue_after_active"}),
        }
        self.dispatch(command).await
    }

    pub async fn implement_plan(
        &self,
        thread_id: &str,
        plan_id: &str,
        markdown: &str,
    ) -> Result<u64> {
        self.set_interaction_mode(thread_id, "default").await?;
        self.dispatch(json!({
            "type": "message.dispatch",
            "threadId": thread_id,
            "messageId": uuid::Uuid::new_v4().to_string(),
            "text": format!("PLEASE IMPLEMENT THIS PLAN:\n{}", markdown.trim()),
            "attachments": [],
            "createdBy": "user",
            "creationSource": "web",
            "deliveryIntent": "auto",
            "dispatchMode": {"type": "start_immediately"},
            "sourcePlanRef": {"threadId": thread_id, "planId": plan_id},
        }))
        .await
    }

    pub async fn cancel_queued(&self, thread_id: &str, run_id: &str) -> Result<u64> {
        self.dispatch(json!({"type": "queued-run.cancel", "threadId": thread_id, "runId": run_id}))
            .await
    }

    pub async fn steer_queued(
        &self,
        thread_id: &str,
        queued_run_id: &str,
        target_run_id: &str,
    ) -> Result<u64> {
        self.dispatch(json!({
            "type": "queued-message.promote-to-steer",
            "threadId": thread_id,
            "queuedRunId": queued_run_id,
            "targetRunId": target_run_id,
        }))
        .await
    }

    pub async fn stop_thread(&self, thread_id: &str) -> Result<u64> {
        self.dispatch(json!({"type": "thread.stop", "threadId": thread_id}))
            .await
    }

    pub async fn visit_thread(&self, thread_id: &str, visited_at: &str) -> Result<u64> {
        self.dispatch(json!({
            "type": "thread.visit",
            "threadId": thread_id,
            "visitedAt": visited_at,
            "opened": true,
        }))
        .await
    }

    pub async fn pin_thread(&self, thread_id: &str, pinned: bool) -> Result<u64> {
        let kind = if pinned { "thread.pin" } else { "thread.unpin" };
        self.dispatch(json!({"type": kind, "threadId": thread_id}))
            .await
    }

    pub async fn snooze_thread(&self, thread_id: &str, until: Option<&str>) -> Result<u64> {
        match until {
            Some(until) => {
                self.dispatch(
                    json!({"type": "thread.snooze", "threadId": thread_id, "snoozedUntil": until}),
                )
                .await
            }
            None => {
                self.dispatch(
                    json!({"type": "thread.unsnooze", "threadId": thread_id, "reason": "user"}),
                )
                .await
            }
        }
    }

    pub async fn search_entries(&self, cwd: &str, query: &str, limit: u64) -> Result<Value> {
        self.rpc()
            .await?
            .call(
                "projects.searchEntries",
                json!({"cwd": cwd, "query": query, "limit": limit}),
                DISPATCH_TIMEOUT,
            )
            .await
            .map_err(|e| match e {
                RpcError::Failed { message, .. } => {
                    anyhow!("T3 could not search the project: {message}")
                }
                other => other.into(),
            })
    }

    /// Uploads a file for a message and returns the attachment the message refers to it by.
    pub async fn upload_attachment(
        &self,
        name: &str,
        mime_type: &str,
        bytes: Vec<u8>,
    ) -> Result<Value> {
        let kind = if mime_type.starts_with("image/") {
            "image"
        } else {
            "file"
        };
        let size = bytes.len();
        let issued = self
            .rpc()
            .await?
            .call(
                "attachments.createUploadUrl",
                json!({"type": kind, "name": name, "mimeType": mime_type, "sizeBytes": size}),
                DISPATCH_TIMEOUT,
            )
            .await
            .map_err(|e| match e {
                RpcError::Failed { message, .. } => {
                    anyhow!("T3 refused the attachment {name}: {message}")
                }
                other => other.into(),
            })?;
        let id = issued
            .get("attachmentId")
            .and_then(Value::as_str)
            .context("T3 returned no attachment id")?;
        let url = issued
            .get("relativeUrl")
            .and_then(Value::as_str)
            .context("T3 returned no upload URL")?;
        self.api.post_bytes(url, mime_type, bytes).await?;
        Ok(json!({"type": kind, "id": id, "name": name, "mimeType": mime_type, "sizeBytes": size}))
    }

    pub async fn archive_thread(&self, thread_id: &str) -> Result<u64> {
        self.dispatch(json!({"type": "thread.archive", "threadId": thread_id}))
            .await
    }

    pub async fn settle_thread(&self, thread_id: &str, settled: bool) -> Result<u64> {
        if settled {
            self.dispatch(json!({"type": "thread.settle", "threadId": thread_id}))
                .await
        } else {
            self.dispatch(
                json!({"type": "thread.unsettle", "threadId": thread_id, "reason": "user"}),
            )
            .await
        }
    }

    pub async fn respond(&self, thread_id: &str, request_id: &str, decision: &str) -> Result<u64> {
        self.dispatch(json!({
            "type": "runtime-request.respond",
            "threadId": thread_id,
            "requestId": request_id,
            "decision": decision,
        }))
        .await
    }

    pub async fn answer(&self, thread_id: &str, request_id: &str, answers: Value) -> Result<u64> {
        self.dispatch(json!({
            "type": "runtime-request.respond",
            "threadId": thread_id,
            "requestId": request_id,
            "answers": answers,
        }))
        .await
    }

    pub async fn set_model(
        &self,
        thread_id: &str,
        model: Value,
        switch_provider: bool,
    ) -> Result<u64> {
        let kind = if switch_provider {
            "provider.switch"
        } else {
            "thread.model-selection.set"
        };
        self.dispatch(json!({"type": kind, "threadId": thread_id, "modelSelection": model}))
            .await
    }

    pub async fn set_runtime_mode(&self, thread_id: &str, runtime_mode: &str) -> Result<u64> {
        self.dispatch(json!({"type": "thread.runtime-mode.set", "threadId": thread_id, "runtimeMode": runtime_mode}))
            .await
    }

    pub async fn set_interaction_mode(
        &self,
        thread_id: &str,
        interaction_mode: &str,
    ) -> Result<u64> {
        self.dispatch(json!({"type": "thread.interaction-mode.set", "threadId": thread_id, "interactionMode": interaction_mode}))
            .await
    }

    pub fn spawn_config(self: &Arc<Self>, runtime: &Handle) -> JoinHandle<()> {
        let connection = self.clone();
        runtime.spawn(async move { connection.follow_config().await })
    }

    async fn follow_config(self: Arc<Self>) {
        let mut retry = RETRY_FIRST;
        loop {
            if self.config_once(&mut retry).await.is_ok() {
                retry = RETRY_FIRST;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(RETRY_LIMIT);
        }
    }

    async fn config_once(&self, retry: &mut Duration) -> Result<()> {
        let rpc = self.rpc().await?;
        let mut stream = rpc.stream("subscribeServerConfig", json!({})).await?;
        while let Some(item) = stream.next().await {
            let item = item?;
            let applied = apply_config(&mut self.config.lock().expect("config lock"), &item);
            if applied == Applied::Snapshot {
                *retry = RETRY_FIRST;
            }
            if !matches!(applied, Applied::Ignored) {
                self.notify(Change::Config {
                    environment: self.label.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn spawn_shell(self: &Arc<Self>, runtime: &Handle) -> JoinHandle<()> {
        let connection = self.clone();
        runtime.spawn(async move { connection.follow_shell().await })
    }

    async fn follow_shell(self: Arc<Self>) {
        let mut retry = RETRY_FIRST;
        loop {
            self.set_status(ConnectionStatus::Connecting);
            match self.shell_once(&mut retry).await {
                Ok(()) => self.set_status(ConnectionStatus::Disconnected(
                    "the shell stream ended".into(),
                )),
                Err(error) => self.set_status(ConnectionStatus::Disconnected(format!("{error:#}"))),
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(RETRY_LIMIT);
        }
    }

    async fn shell_once(&self, retry: &mut Duration) -> Result<()> {
        if self.environment_id().is_none() {
            self.describe().await?;
        }
        let rpc = self.rpc().await?;
        let after = self.shell.lock().expect("shell lock").sequence;
        let mut payload = json!({"requestCompletionMarker": true});
        if after > 0 {
            payload["afterSequence"] = json!(after);
        }
        let mut stream = rpc.stream("orchestration.subscribeShell", payload).await?;
        while let Some(item) = stream.next().await {
            let item = item?;
            let applied = self.shell.lock().expect("shell lock").apply(&item);
            if applied == Applied::Synchronized {
                self.set_status(ConnectionStatus::Connected);
                *retry = RETRY_FIRST;
            }
            if !matches!(applied, Applied::Duplicate | Applied::Ignored) {
                self.notify(Change::Shell {
                    environment: self.label.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn watch_thread(self: &Arc<Self>, runtime: &Handle, thread_id: &str) -> ThreadWatch {
        let state = Arc::new(Mutex::new(ThreadState::default()));
        let connection = self.clone();
        let thread_id = thread_id.to_string();
        let task = runtime.spawn({
            let state = state.clone();
            let thread_id = thread_id.clone();
            async move { connection.follow_thread(&thread_id, state).await }
        });
        ThreadWatch {
            thread_id,
            state,
            task,
        }
    }

    async fn follow_thread(&self, thread_id: &str, state: Arc<Mutex<ThreadState>>) {
        let mut retry = RETRY_FIRST;
        loop {
            if self
                .thread_once(thread_id, &state, &mut retry)
                .await
                .is_ok()
            {
                retry = RETRY_FIRST;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(RETRY_LIMIT);
        }
    }

    async fn thread_once(
        &self,
        thread_id: &str,
        state: &Arc<Mutex<ThreadState>>,
        retry: &mut Duration,
    ) -> Result<()> {
        let rpc = self.rpc().await?;
        let after = state.lock().expect("thread lock").sequence;
        let mut payload = json!({"threadId": thread_id, "requestCompletionMarker": true});
        if after > 0 {
            payload["afterSequence"] = json!(after);
        }
        let mut stream = rpc.stream("orchestration.subscribeThread", payload).await?;
        while let Some(item) = stream.next().await {
            let item = item?;
            let applied = state.lock().expect("thread lock").apply(&item);
            if applied == Applied::Synchronized {
                *retry = RETRY_FIRST;
            }
            if !matches!(applied, Applied::Duplicate | Applied::Ignored) {
                self.notify(Change::Thread {
                    environment: self.label.clone(),
                    thread_id: thread_id.to_string(),
                });
            }
        }
        Ok(())
    }
}

pub struct ThreadWatch {
    thread_id: String,
    state: Arc<Mutex<ThreadState>>,
    task: JoinHandle<()>,
}

impl ThreadWatch {
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn state(&self) -> Arc<Mutex<ThreadState>> {
        self.state.clone()
    }
}

impl Drop for ThreadWatch {
    fn drop(&mut self) {
        self.task.abort();
    }
}
