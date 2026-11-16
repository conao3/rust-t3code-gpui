use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use t3_client::rpc::StreamLimits;
use t3_client::session::Connection;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub environment: String,
    pub cwd: String,
    pub include_remote: bool,
}

pub struct Update {
    pub target: Target,
    pub event: Result<Value, String>,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Status {
    pub local: Value,
    pub remote: Value,
    pub error: Option<String>,
}

impl Status {
    pub fn apply(&mut self, event: Result<Value, String>) -> bool {
        let previous = self.clone();
        match event {
            Ok(event) => {
                self.error = None;
                match event.get("_tag").and_then(Value::as_str) {
                    Some("snapshot") => {
                        self.local = event["local"].clone();
                        self.remote = event["remote"].clone();
                    }
                    Some("localUpdated") => self.local = event["local"].clone(),
                    Some("remoteUpdated") => self.remote = event["remote"].clone(),
                    _ => {}
                }
            }
            Err(error) => self.error = Some(error),
        }
        *self != previous
    }

    pub fn branch(&self) -> Option<&str> {
        self.local.get("refName").and_then(Value::as_str)
    }

    pub fn is_repo(&self) -> bool {
        self.local.get("isRepo").and_then(Value::as_bool) == Some(true)
    }
}

pub struct Watch {
    pub target: Target,
    task: JoinHandle<()>,
}

impl Watch {
    pub fn new(
        connection: Arc<Connection>,
        target: Target,
        runtime: &Handle,
        sender: async_channel::Sender<Update>,
    ) -> Self {
        let watched = target.clone();
        let task = runtime.spawn(async move {
            let mut delay = Duration::from_secs(1);
            loop {
                let result: anyhow::Result<()> = async {
                    let mut stream = connection
                        .subscribe(
                            "subscribeVcsStatus",
                            json!({"cwd": watched.cwd, "includeRemote": watched.include_remote}),
                            StreamLimits::new(64, 8 << 20).expect("nonzero stream limits"),
                        )
                        .await?;
                    while let Some(chunk) = stream.next().await {
                        let chunk = chunk?;
                        for event in chunk.values() {
                            sender
                                .send(Update {
                                    target: watched.clone(),
                                    event: Ok(event.clone()),
                                })
                                .await?;
                        }
                        chunk.accept();
                        delay = Duration::from_secs(1);
                    }
                    anyhow::bail!("Workspace status stream ended")
                }
                .await;
                if let Err(error) = result
                    && sender
                        .send(Update {
                            target: watched.clone(),
                            event: Err(error.to_string()),
                        })
                        .await
                        .is_err()
                {
                    return;
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        });
        Self { target, task }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub details_inline: bool,
    pub inset_end: f32,
}

pub fn layout(width: f32, details_open: bool) -> Layout {
    let card_left = width - 280. - 12.;
    let details_inline = card_left - 32. - 48. >= 640.;
    let lane_right = if details_inline && details_open {
        card_left - 32.
    } else {
        width - 48.
    };
    let chat_width = 736_f32.min(width - 96.).min(lane_right - 48.).max(0.);
    let left = ((width - chat_width) / 2.)
        .min(lane_right - chat_width)
        .max(48.);
    Layout {
        details_inline,
        inset_end: (width - left * 2. - chat_width).max(0.),
    }
}

pub fn editor_label(editor: &str) -> &str {
    match editor {
        "cursor" => "Cursor",
        "trae" => "Trae",
        "kiro" => "Kiro",
        "vscode" => "VS Code",
        "vscode-insiders" => "VS Code Insiders",
        "vscodium" => "VSCodium",
        "zed" => "Zed",
        "antigravity" => "Antigravity",
        "idea" => "IntelliJ IDEA",
        "aqua" => "Aqua",
        "clion" => "CLion",
        "datagrip" => "DataGrip",
        "dataspell" => "DataSpell",
        "goland" => "GoLand",
        "phpstorm" => "PhpStorm",
        "pycharm" => "PyCharm",
        "rider" => "Rider",
        "rubymine" => "RubyMine",
        "rustrover" => "RustRover",
        "webstorm" => "WebStorm",
        "file-manager" => "File Manager",
        other => other,
    }
}

#[derive(Debug, PartialEq)]
pub struct BranchTarget {
    pub cwd: String,
    pub worktree: Option<String>,
    pub reuse: bool,
}

pub fn branch_target(
    project: &str,
    checkout: Option<&str>,
    current: Option<&str>,
    reference: &Value,
) -> BranchTarget {
    if let Some(path) = reference["worktreePath"].as_str() {
        let root = std::path::Path::new(checkout.unwrap_or(project));
        let is_project = if checkout.is_some() {
            root == std::path::Path::new(path)
        } else {
            root.starts_with(path)
        };
        return BranchTarget {
            cwd: path.into(),
            worktree: (!is_project).then(|| path.into()),
            reuse: true,
        };
    }
    let worktree = current.filter(|_| reference["isDefault"].as_bool() != Some(true));
    BranchTarget {
        cwd: worktree.unwrap_or(project).into(),
        worktree: worktree.map(str::to_string),
        reuse: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_selection_preserves_worktrees_and_returns_to_the_project_checkout() {
        let target = branch_target(
            "/repo/app",
            None,
            Some("/worktree"),
            &json!({"worktreePath":"/repo"}),
        );
        assert_eq!(
            target,
            BranchTarget {
                cwd: "/repo".into(),
                worktree: None,
                reuse: true
            }
        );
        let target = branch_target(
            "/repo/app",
            Some("/repo"),
            None,
            &json!({"worktreePath":"/worktree"}),
        );
        assert_eq!(target.worktree.as_deref(), Some("/worktree"));
        assert!(target.reuse);
        let target = branch_target(
            "/repo",
            Some("/repo"),
            Some("/worktree"),
            &json!({"isDefault":true}),
        );
        assert_eq!(
            target,
            BranchTarget {
                cwd: "/repo".into(),
                worktree: None,
                reuse: false
            }
        );
        let target = branch_target(
            "/repo",
            None,
            Some("/worktree"),
            &json!({"isDefault":false}),
        );
        assert_eq!(target.cwd, "/worktree");
        assert_eq!(target.worktree.as_deref(), Some("/worktree"));
    }

    #[test]
    fn local_and_remote_events_preserve_the_other_snapshot() {
        let mut state = Status::default();
        assert!(state.apply(Ok(json!({"_tag":"snapshot", "local":{"isRepo":true,"refName":"main"}, "remote":{"aheadCount":2}}))));
        assert!(state.apply(Ok(
            json!({"_tag":"localUpdated", "local":{"isRepo":true,"refName":"feature"}})
        )));
        assert_eq!(state.branch(), Some("feature"));
        assert!(state.is_repo());
        assert_eq!(state.remote["aheadCount"], 2);
        assert!(state.apply(Err("offline".into())));
        assert_eq!(state.branch(), Some("feature"));
        assert!(state.apply(Ok(json!({"_tag":"remoteUpdated", "remote":null}))));
        assert!(state.remote.is_null());
        assert!(state.error.is_none());
        assert!(!state.apply(Ok(json!({"_tag":"remoteUpdated", "remote":null}))));
    }

    #[test]
    fn the_card_moves_chat_only_when_it_would_overlap() {
        assert_eq!(layout(1144., true).inset_end, 240.);
        assert_eq!(layout(1144., false).inset_end, 0.);
        assert_eq!(layout(2200., true).inset_end, 0.);
        assert!(!layout(1011., true).details_inline);
        assert!(layout(1012., true).details_inline);
        for width in 1300..1800 {
            let current = layout(width as f32, true).inset_end;
            let next = layout((width + 1) as f32, true).inset_end;
            assert!((next - current).abs() <= 1.);
        }
    }
}
