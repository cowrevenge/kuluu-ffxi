use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

use crate::agent_codec;
use crate::state::{AgentCommand, AgentEvent};

pub const AGENT_PIDFILE_NAME: &str = "ffxi-agent.pid";

// kuluu-smlg: macOS $TMPDIR purges have unlinked the live socket mid-run, so
// poll our own path between accepts and re-bind when it vanishes.
const SOCKET_LIVENESS_INTERVAL: Duration = Duration::from_secs(2);

pub fn resolve_listen(arg: &str) -> ResolvedListen {
    if arg.eq_ignore_ascii_case("auto") {
        let tmp = std::env::temp_dir();
        let pid = std::process::id();
        ResolvedListen {
            sock: tmp.join(format!("ffxi-agent-{pid}.sock")),
            pidfile: Some(tmp.join(AGENT_PIDFILE_NAME)),
        }
    } else {
        ResolvedListen {
            sock: PathBuf::from(arg),
            pidfile: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedListen {
    pub sock: PathBuf,

    pub pidfile: Option<PathBuf>,
}

pub async fn serve(
    listen: ResolvedListen,
    cmd_tx: mpsc::Sender<AgentCommand>,
    event_tx: broadcast::Sender<AgentEvent>,
    pause: Option<Arc<AtomicBool>>,
    debug_ctrl: Option<crate::debug_control::SharedDebugControl>,
) -> Result<()> {
    let ResolvedListen { sock, pidfile } = listen;

    let mut listener = bind_listener(&sock).await?;

    eprintln!("agent socket listening on {}", sock.display());
    tracing::info!(path = %sock.display(), "ffxi agent socket listening");

    write_pidfile(pidfile.as_deref(), &sock);

    let _cleanup = SocketCleanup {
        sock: sock.clone(),
        pidfile: pidfile.clone(),
    };

    let mut liveness = tokio::time::interval(SOCKET_LIVENESS_INTERVAL);
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let (stream, _addr) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(err) => {
                    tracing::warn!(error = %err, "agent socket accept failed");
                    continue;
                }
            },
            _ = liveness.tick() => {
                if !sock.exists() {
                    tracing::warn!(path = %sock.display(),
                        "agent socket path vanished from disk; re-binding");
                    match bind_listener(&sock).await {
                        Ok(rebound) => {
                            listener = rebound;
                            // Reclaim the shared pidfile only when it is gone or
                            // still ours: a newer instance's pointer must win.
                            let reclaim = pidfile
                                .as_deref()
                                .filter(|p| !p.exists() || pidfile_bears_our_pid(p));
                            write_pidfile(reclaim, &sock);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, path = %sock.display(),
                                "agent socket re-bind failed; retrying next liveness tick");
                        }
                    }
                }
                continue;
            }
        };
        tracing::info!("agent socket peer connected");
        let (reader, writer) = stream.into_split();
        let cmd_tx = cmd_tx.clone();
        let event_rx = event_tx.subscribe();
        let pause = pause.clone();
        let debug_ctrl = debug_ctrl.clone();

        if let Err(err) =
            agent_codec::run(reader, writer, cmd_tx, event_rx, pause, debug_ctrl).await
        {
            tracing::debug!(error = %err, "agent socket peer ended with error");
        } else {
            tracing::info!("agent socket peer disconnected");
        }
    }
}

async fn bind_listener(sock: &Path) -> Result<UnixListener> {
    if sock.exists() {
        match UnixStream::connect(sock).await {
            Ok(_) => {
                anyhow::bail!(
                    "agent socket {} is already in use (another kuluu is listening); \
                     pick a different `--agent-listen` path or stop the other instance",
                    sock.display()
                );
            }
            Err(_) => {
                let _ = std::fs::remove_file(sock);
            }
        }
    }

    UnixListener::bind(sock).with_context(|| format!("binding agent socket at {}", sock.display()))
}

fn write_pidfile(pidfile: Option<&Path>, sock: &Path) {
    let Some(path) = pidfile else {
        return;
    };
    let body = serde_json::json!({
        "pid": std::process::id(),
        "sock": sock.to_string_lossy(),
    });
    if let Err(err) = std::fs::write(path, body.to_string()) {
        tracing::warn!(error = %err, path = %path.display(),
            "failed to write agent pidfile (continuing without autodiscovery)");
    }
}

fn pidfile_bears_our_pid(path: &Path) -> bool {
    let Ok(body) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) else {
        return false;
    };
    v.get("pid").and_then(serde_json::Value::as_u64) == Some(u64::from(std::process::id()))
}

struct SocketCleanup {
    sock: PathBuf,
    pidfile: Option<PathBuf>,
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sock);
        if let Some(p) = self.pidfile.as_ref() {
            if pidfile_bears_our_pid(p) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str, ext: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "kuluu-session-agent-{label}-{}.{ext}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn serve_rebinds_when_socket_path_vanishes() {
        let sock = temp_path("rebind", "sock");
        let (cmd_tx, _cmd_rx) = mpsc::channel::<AgentCommand>(8);
        let (event_tx, _keep_alive) = broadcast::channel::<AgentEvent>(8);
        let listen = ResolvedListen {
            sock: sock.clone(),
            pidfile: None,
        };
        let _serve = tokio::spawn(serve(listen, cmd_tx, event_tx.clone(), None, None));

        for _ in 0..100 {
            if sock.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(sock.exists(), "serve never bound {}", sock.display());

        let peer = UnixStream::connect(&sock)
            .await
            .expect("connect while live");
        drop(peer);
        for _ in 0..20 {
            let _ = event_tx.send(AgentEvent::Error {
                message: "unstick peer writer".into(),
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        std::fs::remove_file(&sock).expect("remove live socket out from under serve");

        let deadline = std::time::Instant::now() + SOCKET_LIVENESS_INTERVAL * 3;
        let mut reappeared = false;
        while std::time::Instant::now() < deadline {
            if sock.exists() {
                reappeared = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(reappeared, "socket was not re-bound after external removal");

        UnixStream::connect(&sock)
            .await
            .expect("connect after re-bind");
        let _ = std::fs::remove_file(&sock);
    }

    #[test]
    fn socket_cleanup_preserves_foreign_pidfile() {
        let pidfile = temp_path("foreign", "pid");
        let foreign_pid = u64::from(std::process::id()) + 1;
        let body = serde_json::json!({ "pid": foreign_pid, "sock": "/nonexistent.sock" });
        std::fs::write(&pidfile, body.to_string()).expect("write foreign pidfile");

        drop(SocketCleanup {
            sock: temp_path("foreign-sock", "sock"),
            pidfile: Some(pidfile.clone()),
        });

        assert!(
            pidfile.exists(),
            "drop removed a pidfile bearing a foreign pid"
        );
        let _ = std::fs::remove_file(&pidfile);
    }

    #[test]
    fn socket_cleanup_removes_own_pidfile() {
        let pidfile = temp_path("own", "pid");
        let body = serde_json::json!({ "pid": std::process::id(), "sock": "/nonexistent.sock" });
        std::fs::write(&pidfile, body.to_string()).expect("write own pidfile");

        drop(SocketCleanup {
            sock: temp_path("own-sock", "sock"),
            pidfile: Some(pidfile.clone()),
        });

        assert!(!pidfile.exists(), "drop left our own pidfile behind");
    }
}
