#![cfg(unix)]

use std::net::TcpListener;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use t3_client::config::Environment;
use t3_client::session::Connection;
use t3_terminal::session::{Grid, Input, Session, State, Target, close};
use t3_terminal::{Color, Theme};
use tokio::sync::watch;

struct Server {
    child: Child,
    workspace: PathBuf,
    environment: Environment,
    _home: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        let _ = self.child.wait();
    }
}

async fn server() -> Result<Server> {
    let binary = std::env::var("T3_BIN").context("T3_BIN must name the reference server")?;
    let home = tempfile::tempdir()?;
    let base = home.path().join("server");
    let workspace = home.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
    let child = Command::new(&binary)
        .arg("serve")
        .args(["--host", "127.0.0.1", "--port"])
        .arg(port.to_string())
        .arg("--base-dir")
        .arg(&base)
        .arg("--no-browser")
        .arg(&workspace)
        .current_dir(&workspace)
        .env("HOME", home.path())
        .env("SHELL", "/run/current-system/sw/bin/bash")
        .env("PS1", "")
        .env("PROMPT_COMMAND", "")
        .env_remove("BASH_ENV")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut server = Server {
        child,
        workspace,
        environment: Environment {
            label: "terminal-test".into(),
            http_base_url: format!("http://127.0.0.1:{port}"),
            token: String::new(),
        },
        _home: home,
    };
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if client
            .get(format!(
                "{}/.well-known/t3/environment",
                server.environment.http_base_url
            ))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        ensure!(Instant::now() < deadline, "Reference server did not start");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let issued = Command::new(&binary)
        .args([
            "auth",
            "session",
            "issue",
            "--ttl",
            "1h",
            "--label",
            "terminal-test",
            "--token-only",
            "--base-dir",
        ])
        .arg(&base)
        .output()?;
    ensure!(
        issued.status.success(),
        "Could not issue a reference test session"
    );
    server.environment.token = String::from_utf8(issued.stdout)?.trim().to_owned();
    Ok(server)
}

async fn wait(state: &mut watch::Receiver<State>, test: impl Fn(&State) -> bool) -> Result<()> {
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            {
                let current = state.borrow_and_update();
                ensure!(
                    current.error.is_none(),
                    "Terminal error: {}",
                    current.error.as_deref().unwrap_or_default()
                );
                if test(&current) {
                    return Ok(());
                }
            }
            state.changed().await?;
        }
    })
    .await;
    result.with_context(|| {
        format!(
            "Terminal state timed out; rows: {:?}",
            state.borrow().snapshot.as_ref().map(|snapshot| snapshot
                .row_data
                .iter()
                .map(|row| &row.text)
                .collect::<Vec<_>>())
        )
    })?
}

fn colored(state: &State, text: &str, red: bool) -> bool {
    state.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot.row_data.iter().any(|row| {
            row.text.trim_end() == text
                && row.cells.iter().any(|cell| {
                    !cell.text.trim().is_empty()
                        && if red {
                            cell.foreground.r > cell.foreground.g
                        } else {
                            cell.foreground.g > cell.foreground.r
                        }
                })
        })
    })
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs T3_BIN; run with make integration"]
async fn real_pty_large_unicode_paste_observation_and_shared_grid() -> Result<()> {
    let server = server().await?;
    let (changes, _) = async_channel::unbounded();
    let connection = Connection::new(&server.environment, changes);
    let target = Target {
        thread_id: uuid::Uuid::new_v4().to_string(),
        terminal_id: "term-1".into(),
        cwd: server.workspace.to_string_lossy().into_owned(),
        worktree_path: None,
        project_root: server.workspace.to_string_lossy().into_owned(),
        read_only: false,
        restart_if_not_running: false,
    };
    let theme = Theme {
        foreground: Color {
            r: 245,
            g: 245,
            b: 245,
        },
        background: Color::default(),
        cursor: Color {
            r: 180,
            g: 203,
            b: 255,
        },
        palette: None,
    };
    let runtime = tokio::runtime::Handle::current();
    let (session, mut state, _) = Session::new(
        connection.clone(),
        target.clone(),
        Grid::new(80, 24, 8, 18),
        theme.clone(),
        &runtime,
    );
    wait(&mut state, |state| {
        state.connected && state.status == "running" && state.snapshot.is_some()
    })
    .await?;
    session.input(Input::Text(
        "printf '\\r\\033[2K\\033[31mVT_界\\033[0m\\n'\n".into(),
    ))?;
    wait(&mut state, |state| colored(state, "VT_界", true))
        .await
        .context("colored Unicode output")?;
    session.input(Input::Resize(Grid::new(90, 30, 8, 18)))?;
    session.input(Input::Text("stty size > grid.txt; stty -echo; printf '\\r\\033[2K\\033[32mCAT_READY\\033[0m\\n'; cat > large.txt; stty echo; printf '\\r\\033[2K\\033[32mPASTE_DONE\\033[0m\\n'\n".into()))?;
    wait(&mut state, |state| colored(state, "CAT_READY", false))
        .await
        .context("cat ready")?;
    let paste = format!("{}\n", "界".repeat(300)).repeat(100);
    session.input(Input::Paste(paste.clone()))?;
    session.input(Input::Text("\u{4}".into()))?;
    wait(&mut state, |state| colored(state, "PASTE_DONE", false))
        .await
        .context("paste completed")?;
    assert_eq!(
        std::fs::read_to_string(server.workspace.join("large.txt"))?,
        paste
    );
    assert_eq!(
        std::fs::read_to_string(server.workspace.join("grid.txt"))?.trim(),
        "30 90"
    );
    let observer_target = Target {
        read_only: true,
        ..target.clone()
    };
    let (observer, mut observed, _) = Session::new(
        connection.clone(),
        observer_target,
        Grid::new(12, 4, 8, 18),
        theme,
        &runtime,
    );
    wait(&mut observed, |state| colored(state, "PASTE_DONE", false))
        .await
        .context("observer output")?;
    assert_eq!(observed.borrow().grid.cols, 90);
    assert_eq!(observed.borrow().grid.rows, 30);
    assert!(observer.input(Input::Text("unauthorized".into())).is_err());
    observer.input(Input::Resize(Grid::new(10, 2, 9, 20)))?;
    wait(&mut observed, |state| state.grid.cell_width == 9).await?;
    assert_eq!(
        (observed.borrow().grid.cols, observed.borrow().grid.rows),
        (90, 30)
    );
    close(&connection, &target).await?;
    wait(&mut state, |state| state.status == "closed").await?;
    drop(observer);
    drop(session);
    Ok(())
}
