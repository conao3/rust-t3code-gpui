use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use t3_client::config::Environment;
use t3_client::session::{Change, Connection, ConnectionStatus, Launch};

struct Server {
    child: Child,
    origin: String,
    token: String,
    base: std::path::PathBuf,
    _home: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let group = -(self.child.id() as i32);
        unsafe {
            libc::kill(group, libc::SIGTERM);
        }
        let _ = self.child.wait();
    }
}

fn t3_bin() -> String {
    std::env::var("T3_BIN").expect("T3_BIN must name the t3 executable to test against")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn start_server() -> Server {
    let home = tempfile::tempdir().unwrap();
    let base = home.path().join("t3");
    let workspace = home.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("README.md"), "integration\n").unwrap();
    let port = free_port();
    let child = Command::new(t3_bin())
        .arg("serve")
        .arg("--port")
        .arg(port.to_string())
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--base-dir")
        .arg(&base)
        .arg("--no-browser")
        .arg(&workspace)
        .current_dir(&workspace)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start t3 serve");
    let origin = format!("http://127.0.0.1:{port}");
    let http = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let ready = http
            .get(format!("{origin}/.well-known/t3/environment"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "t3 serve did not come up");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let issued = Command::new(t3_bin())
        .args(["auth", "session", "issue", "--ttl", "1h"])
        .args(["--label", "t3-client-test", "--token-only", "--base-dir"])
        .arg(&base)
        .output()
        .expect("issue a session");
    assert!(issued.status.success(), "t3 auth session issue failed");
    let token = String::from_utf8(issued.stdout).unwrap().trim().to_string();
    assert!(!token.is_empty());
    let created = http
        .post(format!("{origin}/api/projects/mutate"))
        .bearer_auth(&token)
        .header("x-t3-orchestration-protocol", "2")
        .json(&json!({
            "type": "project.create",
            "commandId": uuid::Uuid::new_v4().to_string(),
            "projectId": uuid::Uuid::new_v4().to_string(),
            "title": "integration",
            "workspaceRoot": workspace.to_string_lossy(),
        }))
        .send()
        .await
        .expect("create a project");
    assert!(
        created.status().is_success(),
        "project.create: {}",
        created.status()
    );
    Server {
        child,
        origin,
        token,
        base,
        _home: home,
    }
}

async fn wait_for(
    changes: &async_channel::Receiver<Change>,
    mut done: impl FnMut() -> bool,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "timed out waiting for {what}");
        let _ = tokio::time::timeout(left, changes.recv()).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs T3_BIN; run with `make integration`"]
async fn follows_the_shell_and_a_thread_on_a_real_server() {
    let server = start_server().await;
    let (sender, changes) = async_channel::unbounded();
    let connection = Connection::new(
        &Environment {
            label: "test".into(),
            http_base_url: server.origin.clone(),
            token: server.token.clone(),
        },
        sender,
    );
    let runtime = tokio::runtime::Handle::current();
    let _shell = connection.spawn_shell(&runtime);
    let _config = connection.spawn_config(&runtime);

    let shell = connection.shell();
    wait_for(
        &changes,
        || {
            let projects = shell.lock().unwrap().projects.len();
            connection.status() == ConnectionStatus::Connected && projects > 0
        },
        "the shell snapshot with the project",
    )
    .await;
    assert!(connection.environment_id().is_some());
    let config = connection.config();
    wait_for(
        &changes,
        || {
            config
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|c| c["providers"].is_array() && c["keybindings"].is_array())
        },
        "the server config snapshot",
    )
    .await;

    let project_id = shell.lock().unwrap().projects[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let paired = Command::new(t3_bin())
        .args([
            "pair",
            "--ttl",
            "2m",
            "--label",
            "t3-client-pair",
            "--base-dir",
        ])
        .arg(&server.base)
        .output()
        .expect("t3 pair");
    let printed = String::from_utf8_lossy(&paired.stdout);
    let code = printed
        .lines()
        .find_map(|l| l.strip_prefix("Token: "))
        .expect("t3 pair printed a token")
        .trim()
        .to_string();
    let bearer = t3_client::http::Api::new(&server.origin, "")
        .pair(&code, "t3-client-test")
        .await
        .expect("/oauth/token");
    t3_client::http::Api::new(&server.origin, &bearer)
        .get("/api/auth/session")
        .await
        .expect("the paired token authenticates");

    let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    let attachment = connection
        .upload_attachment("shot.png", "image/png", png)
        .await
        .expect("attachments.createUploadUrl and the upload");
    assert_eq!(attachment["type"], "image");
    assert_eq!(attachment["sizeBytes"], 12);
    assert!(attachment["id"].as_str().is_some_and(|id| !id.is_empty()));

    let workspace_root = shell.lock().unwrap().projects[0]["workspaceRoot"]
        .as_str()
        .unwrap()
        .to_string();
    let read = connection
        .read_file(&workspace_root, "README.md")
        .await
        .expect("projects.readFile");
    assert_eq!(read["contents"], "integration\n");
    let found = connection
        .search_entries(&workspace_root, "READ", 10)
        .await
        .expect("projects.searchEntries");
    assert!(
        found["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["path"] == "README.md"),
        "{found}"
    );

    let thread_id = uuid::Uuid::new_v4().to_string();
    connection
        .dispatch(json!({
            "type": "thread.create",
            "createdBy": "user",
            "creationSource": "web",
            "threadId": thread_id,
            "projectId": project_id,
            "title": "integration thread",
            "modelSelection": {"instanceId": "codex", "model": "gpt-5.5"},
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": null,
            "worktreePath": null,
        }))
        .await
        .expect("thread.create");

    let has_thread = |shell: &Arc<Mutex<_>>| {
        let shell: &Arc<Mutex<t3_client::projection::ShellState>> = shell;
        shell
            .lock()
            .unwrap()
            .threads
            .iter()
            .any(|t: &Value| t["id"] == thread_id.as_str())
    };
    wait_for(
        &changes,
        || has_thread(&shell),
        "the new thread in the shell",
    )
    .await;

    let watch = connection.watch_thread(&runtime, &thread_id);
    let state = watch.state();
    wait_for(
        &changes,
        || state.lock().unwrap().synchronized,
        "the thread snapshot",
    )
    .await;
    assert_eq!(state.lock().unwrap().thread_id(), thread_id);
    assert_eq!(
        state.lock().unwrap().thread()["title"],
        "integration thread"
    );

    let before = state.lock().unwrap().sequence;
    connection
        .settle_thread(&thread_id, true)
        .await
        .expect("thread.settle");
    wait_for(
        &changes,
        || {
            let state = state.lock().unwrap();
            state.sequence > before && !state.thread()["settledAt"].is_null()
        },
        "the settle event on the thread stream",
    )
    .await;
    assert_eq!(
        connection.thread_url(&thread_id).unwrap(),
        format!(
            "{}/{}/{thread_id}",
            server.origin,
            connection.environment_id().unwrap()
        )
    );

    let launched_id = uuid::Uuid::new_v4().to_string();
    let rejected = connection
        .launch_thread(Launch {
            thread_id: &launched_id,
            project_id: &project_id,
            title: "launched",
            model: json!({"instanceId": "integration-missing", "model": "none"}),
            runtime_mode: "full-access",
            interaction_mode: "default",
            attachments: Vec::new(),
            text: "hello",
        })
        .await
        .expect_err("a disabled provider instance");
    assert!(
        rejected
            .to_string()
            .starts_with("T3 rejected the new thread: Provider instance"),
        "{rejected:#}"
    );
}
